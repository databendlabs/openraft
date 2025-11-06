use std::borrow::Borrow;
use std::fmt::Display;
use std::fmt::Formatter;

use super::Progress;
use super::vec_progress::VecProgress;
use crate::quorum::QuorumSet;

pub(crate) struct DisplayVecProgress<'a, ID, Ent, Prog, QS, Fmt>
where
    ID: 'static,
    QS: QuorumSet<ID>,
    Fmt: Fn(&mut Formatter<'_>, &ID, &Ent) -> std::fmt::Result,
{
    pub(crate) inner: &'a VecProgress<ID, Ent, Prog, QS>,
    pub(crate) f: Fmt,
}

impl<ID, Ent, Prog, QS, Fmt> Display for DisplayVecProgress<'_, ID, Ent, Prog, QS, Fmt>
where
    ID: PartialEq + 'static,
    Ent: Borrow<Prog>,
    Prog: PartialOrd + Copy,
    QS: QuorumSet<ID>,
    Fmt: Fn(&mut Formatter<'_>, &ID, &Ent) -> std::fmt::Result,
{
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{{")?;
        for (i, item) in self.inner.iter().enumerate() {
            if i > 0 {
                write!(f, ", ")?;
            }
            (self.f)(f, &item.id, &item.val)?;
        }
        write!(f, "}}")?;

        Ok(())
    }
}
