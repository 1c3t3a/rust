use rustc_index::IndexVec;
use rustc_middle::mir::interpret::Scalar;
use rustc_middle::mir::*;
use rustc_middle::ty::{Ty, TyCtxt};
use rustc_session::Session;

use crate::check_pointers::check_pointers;

pub(super) struct CheckNull;

impl<'tcx> crate::MirPass<'tcx> for CheckNull {
    fn is_enabled(&self, sess: &Session) -> bool {
        sess.ub_checks()
    }

    fn run_pass(&self, tcx: TyCtxt<'tcx>, body: &mut Body<'tcx>) {
        check_pointers(tcx, body, &[], insert_null_check);
    }
}

fn insert_null_check<'tcx>(
    tcx: TyCtxt<'tcx>,
    local_decls: &mut IndexVec<Local, LocalDecl<'tcx>>,
    block_data: &mut BasicBlockData<'tcx>,
    pointer: Place<'tcx>,
    _: Ty<'tcx>,
    source_info: SourceInfo,
    new_block: BasicBlock,
) {
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

    // Check if the pointer is null.
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
            Rvalue::BinaryOp(BinOp::Ne, Box::new((Operand::Copy(addr), zero))),
        ))),
    });

    // Set this block's terminator to our assert, continuing to new_block if we pass
    block_data.terminator = Some(Terminator {
        source_info,
        kind: TerminatorKind::Assert {
            cond: Operand::Copy(is_ok),
            expected: true,
            target: new_block,
            msg: Box::new(AssertKind::NullPointerDereference),
            // This calls panic_misaligned_pointer_dereference, which is #[rustc_nounwind].
            // We never want to insert an unwind into unsafe code, because unwinding could
            // make a failing UB check turn into much worse UB when we start unwinding.
            unwind: UnwindAction::Unreachable,
        },
    });
}
