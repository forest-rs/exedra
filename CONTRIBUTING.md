# Contributing to Exedra

Exedra favors small, explicit changes whose invariants and validation are easy
to recover from the repository itself.

## Tickets and plans

Tickets are a coordination aid, not a requirement for every change. Create one
when work spans sessions or contributors, remains blocked or unresolved, has
meaningful dependencies or compatibility impact, establishes a durable public
contract, or is explicitly requested by a maintainer.

A small change that can be implemented, tested, and committed in one session
does not need a ticket. Do not create a ticket merely to mirror every function,
file, test, or commit. Prefer one durable work item with a short checklist over
a tree of micro-tickets. Parent and child tickets are useful only when the
children have independent owners, acceptance criteria, or dependency
boundaries.

Before creating a ticket, check whether an open ticket, ADR, or current roadmap
milestone already owns the work. When closing a ticket, record what changed,
the important invariant or tradeoff, the validation performed, and any
remaining limitation.

Use a short plan or ADR when the decision is architectural and expected to
outlive the implementation. Completed sequencing documents should be marked
historical or consolidated into [`ROADMAP.md`](ROADMAP.md), rather than left
looking like active plans.

## Commit messages

Every implementation commit should stand on its own for a reader without the
conversation that produced it. Use an imperative subject and a body that
explains:

- the subsystem and behavior changed;
- the invariant or reason when it is not obvious;
- relevant compatibility or migration consequences;
- the most important validation and result.

A ticket title is not a substitute for a commit explanation. Ticket IDs may be
included in a body when useful, but should not lead the subject. Keep public
messages, tickets, ADRs, examples, and tests independent of private domains;
prefer neutral terms such as “external frontend,” “recipe,” “part,” “assembly,”
“source reference,” and “material slot.”

## Required validation

Run the smallest deterministic checks while developing and the workspace gates
before handoff when practical:

```sh
typos
taplo fmt --check
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo doc --workspace --no-deps --all-features
```

Foundational crates should also retain their documented `no_std` checks. If a
gate is already failing, record the exact failure and do not describe the
affected capability as complete until it is fixed or bounded explicitly.
