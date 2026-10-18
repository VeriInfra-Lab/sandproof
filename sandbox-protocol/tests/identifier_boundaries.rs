//! Fixed original parser is the regression oracle, not a new production path.
use sandproof_protocol::{
    ClientRequestId, DeploymentId, LogicalActionId, NodeId, OperationId, ResourceId, SandboxId,
    TenantId, parse_allocated_number,
};

fn original_number(value: &str, prefix: &str, highwater: u64) -> Option<u64> {
    let suffix = value.strip_prefix(prefix)?;
    let value = suffix.parse::<u64>().ok()?;
    (value > 0 && value <= highwater && value.to_string() == suffix).then_some(value)
}

#[test]
fn generated_parser_keeps_original_prefix_parse_frontier_and_canonical_checks() {
    let prefixes = ["", "sandbox-", "operation-", "n", "é", "汉", "+", "0"];
    let suffixes = [
        "",
        "0",
        "00",
        "0001",
        "01",
        "1",
        "2",
        "9",
        "10",
        "+0",
        "+1",
        "-1",
        "1 ",
        " 1",
        "1.0",
        "1e0",
        "１",
        "١",
        "\n1",
        "1\0",
        "18446744073709551614",
        "18446744073709551615",
        "18446744073709551616",
        "+18446744073709551615",
    ];
    for prefix in prefixes {
        for suffix in suffixes {
            for highwater in [0, 1, 2, 9, 10, u64::MAX - 1, u64::MAX] {
                let value = format!("{prefix}{suffix}");
                assert_eq!(
                    parse_allocated_number(&value, prefix, highwater),
                    original_number(&value, prefix, highwater),
                    "prefix={prefix:?} suffix={suffix:?} highwater={highwater}"
                );
                for other in ["other-", "é", "汉", "éé", "longer-than-input"] {
                    assert_eq!(
                        parse_allocated_number(&value, other, highwater),
                        original_number(&value, other, highwater)
                    );
                }
            }
        }
    }
    for (value, prefix) in [("é1", "e"), ("汉1", "ab"), ("1", "é"), ("", "n")] {
        assert_eq!(parse_allocated_number(value, prefix, u64::MAX), None);
    }
}

#[test]
fn all_opaque_types_keep_exact_bytes_and_the_same_constructor_domain() {
    macro_rules! check {
        ($kind:ty, $value:expr, $valid:expr) => {{
            let value = $value;
            let owned = <$kind>::try_from(value.to_owned());
            let borrowed = <$kind>::try_from(value);
            assert_eq!(owned.is_ok(), $valid, "{}: {value:?}", stringify!($kind));
            assert_eq!(owned, borrowed);
            if let Ok(id) = owned {
                assert_eq!(id.as_str(), value);
                assert_eq!(String::from(id), value);
            }
        }};
    }
    for (value, valid) in [
        ("", false),
        ("a", true),
        ("A0-z_9.", true),
        ("001", true),
        ("..", true),
        ("/", false),
        ("a b", false),
        ("é", false),
        ("汉", false),
        ("\0", false),
        ("a\nb", false),
        ("a+b", false),
    ] {
        check!(TenantId, value, valid);
        check!(SandboxId, value, valid);
        check!(OperationId, value, valid);
        check!(LogicalActionId, value, valid);
        check!(ResourceId, value, valid);
        check!(ClientRequestId, value, valid);
        check!(DeploymentId, value, valid);
        check!(NodeId, value, valid);
    }
    for length in [127, 128, 129] {
        let value = "a".repeat(length);
        check!(TenantId, value.as_str(), length <= 128);
        check!(NodeId, value.as_str(), length <= 128);
    }
}
