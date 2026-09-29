//! Plan optimization under the strict reference numerical profile.
//!
//! Permitted rewrites preserve every primitive operation's inputs and order:
//! constant folding (evaluating the same f64 operation once), dead pure node
//! elimination, common-subexpression elimination of identical pure nodes, and
//! kernel fusion (see `eval::fuse_groups`). Effects, random draws, receipts,
//! and diagnostics roots are never merged or removed.

use crate::compiler::Plan;

pub fn optimize(mut plan: Plan) -> Plan {
    plan.optimized = true;
    plan
}
