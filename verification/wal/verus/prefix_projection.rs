//! Refinement from EnvironmentWal's three-unit prefixes to EnvironmentCrossLog.
//! This is a model proof. Actual byte framing and native I/O are separate layers.
use vstd::prelude::*;

verus! {
    pub open spec fn permitted(base: nat, part: nat, kept: nat) -> bool {
        part <= 3 && 3 * base <= kept <= 3 * base + part
    }
    pub open spec fn complete(kept: nat) -> nat { kept / 3 }
    pub open spec fn state_at<T>(before: T, pending: T, units: nat) -> T {
        if units == 3 { pending } else { before }
    }
    pub open spec fn states<T>(before: T, pending: T, part: nat) -> ISet<T> {
        ISet::new(|s: T| exists|units: nat| permitted(0, part, units) && s == state_at(before, pending, units))
    }
    pub open spec fn cross_states<T>(durable: T, current: T) -> ISet<T> {
        states(durable, current, if durable == current { 0 } else { 3 })
    }
    pub open spec fn independent<Q, C>(qd: Q, qc: Q, cd: C, cc: C) -> ISet<(Q, C)> {
        ISet::new(|pair: (Q, C)| cross_states(qd, qc).contains(pair.0) && cross_states(cd, cc).contains(pair.1))
    }

    pub proof fn complete_prefix_bound(base: nat, part: nat, kept: nat)
        requires permitted(base, part, kept),
        ensures base <= complete(kept) <= base + 1,
            (complete(kept) == base + 1) == (part == 3 && kept == 3 * base + 3),
            part < 3 ==> complete(kept) == base,
    {
        assert(base <= kept / 3 <= base + 1) by(nonlinear_arith)
            requires part <= 3, 3 * base <= kept <= 3 * base + part;
        assert((kept / 3 == base + 1) == (part == 3 && kept == 3 * base + 3)) by(nonlinear_arith)
            requires part <= 3, 3 * base <= kept <= 3 * base + part;
    }

    pub proof fn truncated_boundary(kept: nat)
        ensures 3 * complete(kept) <= kept < 3 * complete(kept) + 3,
            complete(3 * complete(kept)) == complete(kept),
    {
        assert(3 * (kept / 3) <= kept < 3 * (kept / 3) + 3) by(nonlinear_arith);
        assert((3 * (kept / 3)) / 3 == kept / 3) by(nonlinear_arith);
    }

    pub proof fn project_recovered_prefix<T>(before: T, pending: T, base: nat, part: nat, kept: nat)
        requires permitted(base, part, kept),
        ensures states(before, pending, part).contains(state_at(before, pending, (kept - 3 * base) as nat)),
            complete(kept) == base ==> state_at(before, pending, (kept - 3 * base) as nat) == before,
            complete(kept) > base ==> state_at(before, pending, (kept - 3 * base) as nat) == pending,
    {
        complete_prefix_bound(base, part, kept);
        let tail = (kept - 3 * base) as nat;
        assert(permitted(0, part, tail));
        assert(states(before, pending, part).contains(state_at(before, pending, tail)));
    }

    pub proof fn exact_states<T>(before: T, pending: T, part: nat)
        requires part <= 3,
        ensures states(before, pending, part)
            == if part == 3 { ISet::empty().insert(before).insert(pending) } else { ISet::empty().insert(before) },
    {
        assert forall|s: T| states(before, pending, part).contains(s)
            <==> (s == before || (part == 3 && s == pending)) by {
            if states(before, pending, part).contains(s) {
                let units = choose|units: nat| permitted(0, part, units) && s == state_at(before, pending, units);
                assert(units <= part);
            } else if s == before {
                assert(permitted(0, part, 0) && s == state_at(before, pending, 0));
            } else if part == 3 && s == pending {
                assert(permitted(0, part, 3) && s == state_at(before, pending, 3));
            }
        }
        assert(states(before, pending, part) =~=
            if part == 3 { ISet::empty().insert(before).insert(pending) } else { ISet::empty().insert(before) });
    }

    /// Exactly the set formerly declared directly by EnvironmentCrossLog.
    pub proof fn cross_projection<T>(durable: T, current: T)
        ensures cross_states(durable, current) == ISet::empty().insert(durable).insert(current),
    {
        exact_states(durable, current, if durable == current { 0 } else { 3 });
        if durable == current {
            assert(ISet::empty().insert(durable) =~= ISet::empty().insert(durable).insert(current));
        }
    }

    /// Independent queue/catalog persistence admits both mixed outcomes.
    pub proof fn independent_recovery<Q, C>(qd: Q, qc: Q, cd: C, cc: C)
        ensures independent(qd, qc, cd, cc)
            == ISet::empty().insert((qd, cd)).insert((qd, cc)).insert((qc, cd)).insert((qc, cc)),
            qd != qc && cd != cc ==> independent(qd, qc, cd, cc).contains((qd, cc))
                && !ISet::empty().insert((qd, cd)).insert((qc, cc)).contains((qd, cc)),
    {
        cross_projection(qd, qc); cross_projection(cd, cc);
        assert(independent(qd, qc, cd, cc) =~=
            ISet::empty().insert((qd, cd)).insert((qd, cc)).insert((qc, cd)).insert((qc, cc)));
    }
}
