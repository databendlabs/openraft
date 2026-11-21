use super::ParseSizeError::Empty;
use super::ParseSizeError::InvalidDigit;
use super::ParseSizeError::PosOverflow;
use super::parse_size;
use crate::Config;
use crate::config::error::ConfigError;

/// A mantissa that rounds up from `u64::MAX` is 2^64: too large on its own, and in range after a
/// negative exponent. `parse-size` 1.1.0 overflowed on all four: a panic in a debug build, 0 in a
/// release build.
#[test]
fn test_rounding_a_mantissa_past_u64_max() {
    assert_eq!(Ok(u64::MAX), parse_size("18446744073709551615"));
    assert_eq!(Err(PosOverflow), parse_size("18446744073709551616"));

    assert_eq!(Err(PosOverflow), parse_size("184467440737095516155"));
    assert_eq!(Err(PosOverflow), parse_size("18446744073709551615.5"));
    assert_eq!(Ok(18), parse_size("18446744073709551615.5e-18"));
    assert_eq!(Ok(18), parse_size("184467440737095516155e-19"));
}

/// `Config::build` reports a size that does not fit as a parse error with the original arguments.
#[test]
fn test_build_rejects_an_overflowing_size() {
    let args = ["foo", "--snapshot-max-chunk-size=184467440737095516155"];
    let error = Config::build(&args).unwrap_err();
    let ConfigError::ParseError { source, args: returned } = error else {
        panic!("expected ConfigError::ParseError, got: {error:?}");
    };
    assert_eq!(args, returned.as_slice());
    let reason = source.to_string();
    assert!(reason.contains("number too large to fit in target type"), "{reason}");

    let config = Config::build(&["foo", "--snapshot-max-chunk-size=184467440737095516155e-19"]).unwrap();
    assert_eq!(18, config.snapshot_max_chunk_size);
}

/// The sizes that `parse-size` 1.1.0 parsed correctly parse to the same values.
#[test]
fn test_sizes() {
    assert_eq!(Ok(0), parse_size("0"));
    assert_eq!(Ok(204), parse_size("204"));
    assert_eq!(Ok(5), parse_size("_5_"));
    assert_eq!(Ok(1_234_567), parse_size("1 234 567"));
    assert_eq!(Ok(18_446_744_073_709_551_581), parse_size("18_446_744_073_709_551_581"));
    assert_eq!(Err(PosOverflow), parse_size("18_446_744_073_709_551_620"));

    assert_eq!(Ok(1_000), parse_size("1kB"));
    assert_eq!(Ok(2_000_000), parse_size("2MB"));
    assert_eq!(Ok(3_000_000_000), parse_size("3GB"));
    assert_eq!(Ok(4_000_000_000_000), parse_size("4TB"));
    assert_eq!(Ok(5_000_000_000_000_000), parse_size("5PB"));
    assert_eq!(Ok(6_000_000_000_000_000_000), parse_size("6EB"));
    assert_eq!(Ok(7 << 10), parse_size("7 KiB"));
    assert_eq!(Ok(8 << 20), parse_size("8 MiB"));
    assert_eq!(Ok(9 << 30), parse_size("9 GiB"));
    assert_eq!(Ok(10 << 40), parse_size("10 TiB"));
    assert_eq!(Ok(11 << 50), parse_size("11 PiB"));
    assert_eq!(Ok(12 << 60), parse_size("12 EiB"));
    assert_eq!(Ok(3 * 1024 * 1024), parse_size("3MiB"));
    assert_eq!(Ok(1_048_576), parse_size("1mib"));

    assert_eq!(Ok(5000), parse_size("5k"));
    assert_eq!(Ok(5_300), parse_size("5.3 KB"));
    assert_eq!(Ok(1100), parse_size("1.1 K"));
    assert_eq!(Ok(1235), parse_size("1.2345 K"));
    assert_eq!(Ok(1_234_500), parse_size("1.2345m"));
    assert_eq!(Ok(5000), parse_size("5.k"));
    assert_eq!(Ok(3), parse_size("0.0025KB"));
    assert_eq!(Ok(209_715), parse_size("0.2 MiB"));
    assert_eq!(Ok(3_141_592_653_589_793_238), parse_size("3.141_592_653_589_793_238e"));
    assert_eq!(
        Ok(18_446_744_073_709_551_615),
        parse_size("18.446_744_073_709_551_615 EB")
    );
    assert_eq!(Err(PosOverflow), parse_size("18.446_744_073_709_551_616 EB"));
    assert_eq!(
        Ok(1_000_000_000_000_000_001),
        parse_size("1.000_000_000_000_000_001 EB")
    );

    assert_eq!(Ok(102_400), parse_size("1e2 KIB"));
    assert_eq!(Ok(1_000_000), parse_size("1E+6"));
    assert_eq!(Ok(105), parse_size("1e-4 MiB"));
    assert_eq!(Ok(110), parse_size("1.1e2"));
    assert_eq!(Ok(5700), parse_size("5.7E3"));
    assert_eq!(Ok(20), parse_size("20_000_000_000_000_000_000e-18"));
    assert_eq!(Ok(9877), parse_size("98_765_432_109_876_543_210e-16"));
    assert_eq!(Err(PosOverflow), parse_size("1e21"));
    assert_eq!(Ok(10_000_000_000_000_000_000), parse_size("0.01e21"));
    assert_eq!(
        Ok(3_333_333_333_333_333_333),
        parse_size("3.333_333_333_333_333_333_333_333_333_333_333_333_333_333_333_333 EB")
    );
    assert_eq!(Ok(2_000_000_000), parse_size("2e+0_9"));
    assert_eq!(Err(PosOverflow), parse_size("3e+66666666666666666666"));
    assert_eq!(Ok(0), parse_size("4e-77777777777777777777"));
    assert_eq!(Ok(0), parse_size("5e-88888888888888888888 EiB"));
    assert_eq!(
        Ok(12_345_678_901),
        parse_size("123_456_789_012_345_678_901_234_567.890e-16")
    );
    assert_eq!(Ok(0), parse_size("0.1e-2147483648"));
    assert_eq!(Ok(2), parse_size("184467440737095516150e-38EiB"));
}

/// The inputs that `parse-size` 1.1.0 rejected fail with the same errors and messages.
#[test]
fn test_errors() {
    for (expected, src) in [
        (Empty, ""),
        (InvalidDigit, "."),
        (InvalidDigit, ".5k"),
        (Empty, "k"),
        (Empty, "kb"),
        (Empty, "kib"),
        (Empty, "  "),
        (Empty, "__"),
        (InvalidDigit, "a"),
        (InvalidDigit, "-1"),
        (InvalidDigit, "1,5"),
        (InvalidDigit, "1e+f"),
        (InvalidDigit, "1e0.5"),
        (InvalidDigit, "1 ZiB"),
        (InvalidDigit, "1 YiB"),
    ] {
        assert_eq!(Err(expected), parse_size(src), "input: {src:?}");
    }

    for (expected, std_input) in [(Empty, ""), (InvalidDigit, "?"), (PosOverflow, "99999999999999999999")] {
        let std_error = std_input.parse::<u64>().unwrap_err();
        assert_eq!(std_error.to_string(), expected.to_string());
    }
}
