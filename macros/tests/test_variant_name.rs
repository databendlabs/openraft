//! Behavior of `#[derive(VariantName)]`.
//!
//! The derive emits `COUNT`, `ALL`, `index()`, `as_str()` and `Display` from one enum definition
//! precisely so they cannot disagree. These tests pin each generated item against a full expected
//! value and then assert the agreement between them, since that agreement — not any single item —
//! is what callers rely on when they use `index()` to address a `[T; COUNT]` array.

use openraft_macros::VariantName;

/// A leaf enum whose variants render with a prefix.
#[derive(Debug, Clone, Copy, PartialEq, Eq, VariantName)]
#[variant_name(prefix = "sm::")]
enum SmName {
    Build,
    Apply,
}

/// A leaf enum with no prefix, of a different length than [`SmName`] so the two cannot be
/// confused when both are nested into the same outer enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, VariantName)]
enum ExtName {
    Snapshot,
    Purge,
    Trigger,
}

/// Two nested variants, neither first nor last, so every offset position is covered: before any
/// nesting, between two nestings, and after both.
#[derive(Debug, Clone, Copy, PartialEq, Eq, VariantName)]
#[variant_name(prefix = "outer::")]
enum Outer {
    Vote,
    StateMachine(SmName),
    Respond,
    External(ExtName),
    Tick,
}

/// A nested variant in first position, where the array filler has to come from the inner enum
/// rather than from a unit variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, VariantName)]
enum NestedFirst {
    Sm(SmName),
    Tail,
}

/// The smallest enum the derive accepts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, VariantName)]
enum Solo {
    Only,
}

/// Nesting is transitive: the enum wrapped here already contains nested variants of its own, so
/// its `COUNT` and `ALL` are themselves expansions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, VariantName)]
#[variant_name(prefix = "deep::")]
enum Deep {
    Head,
    Nested(Outer),
    Tail,
}

/// `index()` addresses a fixed-size array, so the indices must be exactly `0..COUNT` in `ALL`
/// order. Nothing in the generated code ties `COUNT`, `ALL` and `index()` together; this checks it.
macro_rules! assert_dense_index {
    ($($ty:ty),* $(,)?) => {$({
        assert_eq!(
            <$ty>::ALL.len(),
            <$ty>::COUNT,
            "{}: ALL must hold exactly COUNT variants",
            stringify!($ty)
        );

        for (i, variant) in <$ty>::ALL.iter().enumerate() {
            assert_eq!(
                variant.index(),
                i,
                "{}: {:?} must report its own position in ALL",
                stringify!($ty),
                variant
            );
        }
    })*};
}

#[test]
fn count_counts_a_nested_variant_as_the_whole_inner_enum() {
    assert_eq!(SmName::COUNT, 2);
    assert_eq!(ExtName::COUNT, 3);
    assert_eq!(Solo::COUNT, 1);

    // 3 unit variants, plus SmName's 2 and ExtName's 3.
    assert_eq!(Outer::COUNT, 8);
    assert_eq!(NestedFirst::COUNT, 3);
}

#[test]
fn all_expands_a_nested_variant_in_place() {
    assert_eq!(SmName::ALL, &[SmName::Build, SmName::Apply]);
    assert_eq!(ExtName::ALL, &[ExtName::Snapshot, ExtName::Purge, ExtName::Trigger]);
    assert_eq!(Solo::ALL, &[Solo::Only]);

    assert_eq!(Outer::ALL, &[
        Outer::Vote,
        Outer::StateMachine(SmName::Build),
        Outer::StateMachine(SmName::Apply),
        Outer::Respond,
        Outer::External(ExtName::Snapshot),
        Outer::External(ExtName::Purge),
        Outer::External(ExtName::Trigger),
        Outer::Tick,
    ]);

    assert_eq!(NestedFirst::ALL, &[
        NestedFirst::Sm(SmName::Build),
        NestedFirst::Sm(SmName::Apply),
        NestedFirst::Tail,
    ]);
}

#[test]
fn index_equals_position_in_all() {
    assert_dense_index!(SmName, ExtName, Solo, Outer, NestedFirst, Deep);
}

#[test]
fn nesting_is_transitive() {
    assert_eq!(Deep::COUNT, 2 + Outer::COUNT);

    assert_eq!(Deep::Head.index(), 0);
    assert_eq!(Deep::Nested(Outer::Vote).index(), 1);
    assert_eq!(Deep::Nested(Outer::Tick).index(), 1 + Outer::Tick.index());
    assert_eq!(Deep::Tail.index(), 1 + Outer::COUNT);

    // The rendering of a twice-nested variant comes from the innermost enum, so neither the
    // `deep::` nor the `outer::` prefix reaches it.
    assert_eq!(Deep::Head.as_str(), "deep::Head");
    assert_eq!(Deep::Nested(Outer::Vote).as_str(), "outer::Vote");
    assert_eq!(Deep::Nested(Outer::StateMachine(SmName::Apply)).as_str(), "sm::Apply");
    assert_eq!(Deep::Nested(Outer::External(ExtName::Purge)).as_str(), "Purge");

    // `ALL` splices in the inner enum's own expansion rather than one entry per inner variant.
    let inner = &Deep::ALL[1..1 + Outer::COUNT];
    let expected = Outer::ALL.iter().map(|v| Deep::Nested(*v)).collect::<Vec<_>>();
    assert_eq!(inner, expected);
}

#[test]
fn index_after_a_nested_variant_skips_the_whole_inner_enum() {
    // Written against the inner `COUNT`s rather than literals: these relationships are what must
    // survive an inner enum gaining a variant.
    assert_eq!(Outer::Vote.index(), 0);
    assert_eq!(Outer::StateMachine(SmName::Build).index(), 1);
    assert_eq!(Outer::StateMachine(SmName::Apply).index(), 1 + SmName::Apply.index());
    assert_eq!(Outer::Respond.index(), 1 + SmName::COUNT);
    assert_eq!(Outer::External(ExtName::Snapshot).index(), 2 + SmName::COUNT);
    assert_eq!(
        Outer::External(ExtName::Trigger).index(),
        2 + SmName::COUNT + ExtName::Trigger.index()
    );
    assert_eq!(Outer::Tick.index(), 2 + SmName::COUNT + ExtName::COUNT);

    assert_eq!(NestedFirst::Sm(SmName::Build).index(), 0);
    assert_eq!(NestedFirst::Tail.index(), SmName::COUNT);
}

#[test]
fn as_str_prefixes_unit_variants_and_delegates_nested_ones() {
    let names = |all: &[SmName]| all.iter().map(|v| v.as_str()).collect::<Vec<_>>();
    assert_eq!(names(SmName::ALL), ["sm::Build", "sm::Apply"]);

    let names = |all: &[ExtName]| all.iter().map(|v| v.as_str()).collect::<Vec<_>>();
    assert_eq!(names(ExtName::ALL), ["Snapshot", "Purge", "Trigger"]);

    // A nested variant keeps the inner enum's rendering, so it carries the inner prefix and not
    // the outer one.
    let names = |all: &[Outer]| all.iter().map(|v| v.as_str()).collect::<Vec<_>>();
    assert_eq!(names(Outer::ALL), [
        "outer::Vote",
        "sm::Build",
        "sm::Apply",
        "outer::Respond",
        "Snapshot",
        "Purge",
        "Trigger",
        "outer::Tick",
    ]);

    assert_eq!(Solo::Only.as_str(), "Only");
}

#[test]
fn display_renders_as_str() {
    assert_eq!(Outer::Vote.to_string(), "outer::Vote");
    assert_eq!(Outer::StateMachine(SmName::Apply).to_string(), "sm::Apply");
    assert_eq!(Outer::External(ExtName::Purge).to_string(), "Purge");
    assert_eq!(Solo::Only.to_string(), "Only");

    for variant in Outer::ALL {
        assert_eq!(variant.to_string(), variant.as_str());
    }
}

#[test]
fn generated_items_are_const() {
    const COUNT: usize = Outer::COUNT;
    const ALL: &[Outer] = Outer::ALL;
    const UNIT_INDEX: usize = Outer::Tick.index();
    const NESTED_INDEX: usize = Outer::External(ExtName::Purge).index();
    const UNIT_NAME: &str = Outer::Tick.as_str();
    const NESTED_NAME: &str = Outer::StateMachine(SmName::Build).as_str();

    assert_eq!(COUNT, 8);
    assert_eq!(ALL.len(), 8);
    assert_eq!(UNIT_INDEX, 7);
    assert_eq!(NESTED_INDEX, 5);
    assert_eq!(UNIT_NAME, "outer::Tick");
    assert_eq!(NESTED_NAME, "sm::Build");
}

/// `index()` is generated to address an array of exactly `COUNT` counters, which is the reason the
/// three items have to agree. This walks that use case end to end.
#[test]
fn index_addresses_a_count_sized_array() {
    let mut counters = [0usize; Outer::COUNT];

    for variant in Outer::ALL {
        counters[variant.index()] += 1;
    }

    assert_eq!(counters, [1; Outer::COUNT]);
}

#[test]
fn fail() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/variant_name/fail/*.rs");
}
