use rustc_index::IndexVec;
use rustc_middle::mir::interpret::Scalar;
use rustc_middle::mir::*;
use rustc_middle::ty::{Ty, TyCtxt};
use rustc_session::Session;

use crate::check_pointers::check_pointers;

pub(super) struct CheckAlignment;

impl<'tcx> crate::MirPass<'tcx> for CheckAlignment {
    fn is_enabled(&self, sess: &Session) -> bool {
        // FIXME(#112480) MSVC and rustc disagree on minimum stack alignment on x86 Windows
        if sess.target.llvm_target == "i686-pc-windows-msvc" {
            return false;
        }
        sess.ub_checks()
    }

    fn run_pass(&self, tcx: TyCtxt<'tcx>, body: &mut Body<'tcx>) {
        // Skip trivially aligned place types.
        let excluded_pointees = [tcx.types.bool, tcx.types.i8, tcx.types.u8];
        check_pointers(tcx, body, &excluded_pointees, insert_alignment_check);
    }
}

fn insert_alignment_check<'tcx>(
    tcx: TyCtxt<'tcx>,
    local_decls: &mut IndexVec<Local, LocalDecl<'tcx>>,
    block_data: &mut BasicBlockData<'tcx>,
    pointer: Place<'tcx>,
    pointee_ty: Ty<'tcx>,
    source_info: SourceInfo,
    new_block: BasicBlock,
) {
    // Cast the pointer to a *const ()
    let const_raw_ptr = Ty::new_imm_ptr(tcx, tcx.types.unit);
    let rvalue = Rvalue::Cast(CastKind::PtrToPtr, Operand::Copy(pointer), const_raw_ptr);
    let thin_ptr = local_decls.push(LocalDecl::with_source_info(const_raw_ptr, source_info)).into();
    block_data
        .statements
        .push(Statement { source_info, kind: StatementKind::Assign(Box::new((thin_ptr, rvalue))) });

    // Transmute the pointer to a usize (equivalent to `ptr.addr()`)
    let rvalue = Rvalue::Cast(CastKind::Transmute, Operand::Copy(thin_ptr), tcx.types.usize);
    let addr = local_decls.push(LocalDecl::with_source_info(tcx.types.usize, source_info)).into();
    block_data
        .statements
        .push(Statement { source_info, kind: StatementKind::Assign(Box::new((addr, rvalue))) });

    // Get the alignment of the pointee
    let alignment =
        local_decls.push(LocalDecl::with_source_info(tcx.types.usize, source_info)).into();
    let rvalue = Rvalue::NullaryOp(NullOp::AlignOf, pointee_ty);
    block_data.statements.push(Statement {
        source_info,
        kind: StatementKind::Assign(Box::new((alignment, rvalue))),
    });

    // Subtract 1 from the alignment to get the alignment mask
    let alignment_mask =
        local_decls.push(LocalDecl::with_source_info(tcx.types.usize, source_info)).into();
    let one = Operand::Constant(Box::new(ConstOperand {
        span: source_info.span,
        user_ty: None,
        const_: Const::Val(ConstValue::Scalar(Scalar::from_target_usize(1, &tcx)), tcx.types.usize),
    }));
    block_data.statements.push(Statement {
        source_info,
        kind: StatementKind::Assign(Box::new((
            alignment_mask,
            Rvalue::BinaryOp(BinOp::Sub, Box::new((Operand::Copy(alignment), one))),
        ))),
    });

    // BitAnd the alignment mask with the pointer
    let alignment_bits =
        local_decls.push(LocalDecl::with_source_info(tcx.types.usize, source_info)).into();
    block_data.statements.push(Statement {
        source_info,
        kind: StatementKind::Assign(Box::new((
            alignment_bits,
            Rvalue::BinaryOp(
                BinOp::BitAnd,
                Box::new((Operand::Copy(addr), Operand::Copy(alignment_mask))),
            ),
        ))),
    });

    // Check if the alignment bits are all zero
    let is_ok = local_decls.push(LocalDecl::with_source_info(tcx.types.bool, source_info)).into();
    let zero = Operand::Constant(Box::new(ConstOperand {
        span: source_info.span,
        user_ty: None,
        const_: Const::Val(ConstValue::Scalar(Scalar::from_target_usize(0, &tcx)), tcx.types.usize),
    }));
    block_data.statements.push(Statement {
        source_info,
        kind: StatementKind::Assign(Box::new((
            is_ok,
            Rvalue::BinaryOp(BinOp::Eq, Box::new((Operand::Copy(alignment_bits), zero.clone()))),
        ))),
    });

    // Set this block's terminator to our assert, continuing to new_block if we pass
    block_data.terminator = Some(Terminator {
        source_info,
        kind: TerminatorKind::Assert {
            cond: Operand::Copy(is_ok),
            expected: true,
            target: new_block,
            msg: Box::new(AssertKind::MisalignedPointerDereference {
                required: Operand::Copy(alignment),
                found: Operand::Copy(addr),
            }),
            // This calls panic_misaligned_pointer_dereference, which is #[rustc_nounwind].
            // We never want to insert an unwind into unsafe code, because unwinding could
            // make a failing UB check turn into much worse UB when we start unwinding.
            unwind: UnwindAction::Unreachable,
        },
    });
}
