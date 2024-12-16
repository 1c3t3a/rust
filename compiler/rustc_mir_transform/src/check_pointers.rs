use rustc_hir::lang_items::LangItem;
use rustc_index::IndexVec;
use rustc_middle::mir::visit::{MutatingUseContext, NonMutatingUseContext, PlaceContext, Visitor};
use rustc_middle::mir::*;
use rustc_middle::ty::{self, Ty, TyCtxt};
use tracing::{debug, trace};

pub(crate) fn check_pointers<'a, 'tcx, F>(
    tcx: TyCtxt<'tcx>,
    body: &mut Body<'tcx>,
    excluded_pointees: &'a [Ty<'tcx>],
    on_finding: F,
) where
    F: Fn(
        TyCtxt<'tcx>,
        &mut IndexVec<Local, LocalDecl<'tcx>>,
        &mut BasicBlockData<'tcx>,
        Place<'tcx>,
        Ty<'tcx>,
        SourceInfo,
        BasicBlock,
    ),
{
    // This pass emits new panics. If for whatever reason we do not have a panic
    // implementation, running this pass may cause otherwise-valid code to not compile.
    if tcx.lang_items().get(LangItem::PanicImpl).is_none() {
        return;
    }

    let typing_env = body.typing_env(tcx);
    let basic_blocks = body.basic_blocks.as_mut();
    let local_decls = &mut body.local_decls;

    // This operation inserts new blocks. Each insertion changes the Location for all
    // statements/blocks after. Iterating or visiting the MIR in order would require updating
    // our current location after every insertion. By iterating backwards, we dodge this issue:
    // The only Locations that an insertion changes have already been handled.
    for block in (0..basic_blocks.len()).rev() {
        let block = block.into();
        for statement_index in (0..basic_blocks[block].statements.len()).rev() {
            let location = Location { block, statement_index };
            let statement = &basic_blocks[block].statements[statement_index];
            let source_info = statement.source_info;

            let mut finder = PointerFinder::new(tcx, local_decls, typing_env, excluded_pointees);
            finder.visit_statement(statement, location);

            for (local, ty) in finder.into_found_pointers() {
                debug!("Inserting check for {:?}", ty);
                let new_block = split_block(basic_blocks, location);
                on_finding(
                    tcx,
                    local_decls,
                    &mut basic_blocks[block],
                    local,
                    ty,
                    source_info,
                    new_block,
                );
            }
        }
    }
}

struct PointerFinder<'a, 'tcx> {
    tcx: TyCtxt<'tcx>,
    local_decls: &'a mut LocalDecls<'tcx>,
    typing_env: ty::TypingEnv<'tcx>,
    pointers: Vec<(Place<'tcx>, Ty<'tcx>)>,
    excluded_pointees: &'a [Ty<'tcx>],
}

impl<'a, 'tcx> PointerFinder<'a, 'tcx> {
    fn new(
        tcx: TyCtxt<'tcx>,
        local_decls: &'a mut LocalDecls<'tcx>,
        typing_env: ty::TypingEnv<'tcx>,
        excluded_pointees: &'a [Ty<'tcx>],
    ) -> Self {
        PointerFinder { tcx, local_decls, typing_env, excluded_pointees, pointers: Vec::new() }
    }

    fn into_found_pointers(self) -> Vec<(Place<'tcx>, Ty<'tcx>)> {
        self.pointers
    }
}

impl<'a, 'tcx> Visitor<'tcx> for PointerFinder<'a, 'tcx> {
    fn visit_place(&mut self, place: &Place<'tcx>, context: PlaceContext, location: Location) {
        // We want to only check reads and writes to Places, so we specifically exclude
        // Borrow and RawBorrow.
        match context {
            PlaceContext::MutatingUse(
                MutatingUseContext::Store
                | MutatingUseContext::AsmOutput
                | MutatingUseContext::Call
                | MutatingUseContext::Yield
                | MutatingUseContext::Drop,
            ) => {}
            PlaceContext::NonMutatingUse(
                NonMutatingUseContext::Copy | NonMutatingUseContext::Move,
            ) => {}
            _ => {
                return;
            }
        }

        if !place.is_indirect() {
            return;
        }

        // Since Deref projections must come first and only once, the pointer for an indirect place
        // is the Local that the Place is based on.
        let pointer = Place::from(place.local);
        let pointer_ty = self.local_decls[place.local].ty;

        // We only want to check places based on unsafe pointers
        if !pointer_ty.is_unsafe_ptr() {
            trace!("Indirect, but not based on an unsafe ptr, not checking {:?}", place);
            return;
        }

        let pointee_ty =
            pointer_ty.builtin_deref(true).expect("no builtin_deref for an unsafe pointer");
        // Ideally we'd support this in the future, but for now we are limited to sized types.
        if !pointee_ty.is_sized(self.tcx, self.typing_env) {
            debug!("Unsafe pointer, but pointee is not known to be sized: {:?}", pointer_ty);
            return;
        }

        // We don't need to look for str and slices, we already rejected unsized types above
        let element_ty = match pointee_ty.kind() {
            ty::Array(ty, _) => *ty,
            _ => pointee_ty,
        };
        if self.excluded_pointees.contains(&element_ty) {
            debug!("Skipping pointer for type: {:?}", pointee_ty);
            return;
        }

        self.pointers.push((pointer, pointee_ty));

        self.super_place(place, context, location);
    }
}

fn split_block(
    basic_blocks: &mut IndexVec<BasicBlock, BasicBlockData<'_>>,
    location: Location,
) -> BasicBlock {
    let block_data = &mut basic_blocks[location.block];

    // Drain every statement after this one and move the current terminator to a new basic block
    let new_block = BasicBlockData {
        statements: block_data.statements.split_off(location.statement_index),
        terminator: block_data.terminator.take(),
        is_cleanup: block_data.is_cleanup,
    };

    basic_blocks.push(new_block)
}
