use super::*;
use std::{
    fmt::Write as _,
    io::{BufRead, BufReader, Write},
    process::{Child, ChildStdin, ChildStdout, Command, Stdio},
};

struct Reference {
    child: Child,
    input: Option<ChildStdin>,
    output: BufReader<ChildStdout>,
}
impl Reference {
    fn new() -> Self {
        let mut child = Command::new(
            std::env::var_os("SANDPROOF_GUEST_DOWNLOAD_REFERENCE")
                .expect("fixed Go reference required"),
        )
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap();
        Self {
            input: child.stdin.take(),
            output: BufReader::new(child.stdout.take().unwrap()),
            child,
        }
    }
    fn check(&mut self, request: &str, observed: &str) {
        let input = self.input.as_mut().unwrap();
        writeln!(input, "{request}").unwrap();
        input.flush().unwrap();
        let mut line = String::new();
        assert!(self.output.read_line(&mut line).unwrap() > 0);
        assert_eq!(
            observed,
            line.strip_suffix('\n').unwrap(),
            "oracle input {request}"
        );
    }
    fn finish(&mut self) {
        drop(self.input.take());
        assert!(self.child.wait().unwrap().success());
    }
}
impl Drop for Reference {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().fold(String::new(), |mut result, b| {
        write!(result, "{b:02x}").unwrap();
        result
    })
}

#[test]
#[ignore = "explicit Go 1.26.8 LoadLocationFromTZData/ParseTime oracle; set SANDPROOF_GUEST_DOWNLOAD_REFERENCE"]
fn fixed_go_tzif_lookup_and_http_dates() {
    let mut oracle = Reference::new();
    let mut cases = historical();
    for version in *b"23" {
        cases.push(extended(version, true));
        cases.push(extended(version, false));
        cases.push(tzif(
            version,
            &[Type(36000, false, "AEST"), Type(39600, true, "AEDT")],
            &[],
            &[],
            "AEST-10AEDT-11,M10.1.0,M4.1.0/3",
        ));
        cases.push(tzif(
            version,
            &[Type(0, false, "STD"), Type(3600, true, "DST")],
            &[],
            &[],
            "STD0DST,J60/1,J300/2",
        ));
        cases.push(tzif(
            version,
            &[Type(3600, false, "XYZ"), Type(7200, true, "XYZ")],
            &[(instant(1900, 1, 1), 1), (instant(2040, 1, 1), 0)],
            &[],
            "",
        ));
    }
    let mut times = vec![
        -10_000, -1, 0, 1, 99, 100, 101, 199, 200, 201, 999, 1000, 1001, 9999, 10000, 10001,
        78_796_799, 78_796_800, 78_796_809, 78_796_810,
    ];
    for year in [
        1900, 1969, 1970, 1994, 2019, 2020, 2038, 2040, 2050, 2068, 2100,
    ] {
        for month in 1..=12 {
            times.push(instant(year, month, 1));
        }
    }
    // Both sides of spring/fall transitions and the duplicated civil hour.
    for day in [8, 9, 10, 11] {
        for hour in 0..24 {
            times.push(instant(2050, 3, day) + hour * 3600);
        }
    }
    for day in [5, 6, 7, 8] {
        for hour in 0..24 {
            times.push(instant(2050, 11, day) + hour * 3600);
        }
    }
    let mut lookups = 0;
    let mut dates = 0;
    for data in cases {
        let zone = Snapshot::from_tzif(&data, limits()).unwrap();
        let encoded = hex(&data);
        for seconds in &times {
            let local = zone.lookup(*seconds);
            oracle.check(
                &format!("L\t{encoded}\t{seconds}"),
                &format!(
                    "{}\t{}",
                    hex(local.time_zone_designation().as_bytes()),
                    local.ut_offset()
                ),
            );
            lookups += 1;
        }
        for year in [69, 70, 94, 19, 20, 38, 40, 50, 68] {
            for month in ["Jan", "Mar", "Jul", "Nov"] {
                for name in [
                    "AAA", "BBB", "CCC", "XYZ", "STD", "DST", "EST", "EDT", "OLD", "UTC", "GMT",
                    "ABC", "AEST", "AEDT",
                ] {
                    for fraction in ["", ".123456789"] {
                        let date =
                            format!("Sunday, 06-{month}-{year:02} 02:30:00{fraction} {name}");
                        let observed = date::parse(date.as_bytes(), &zone).map_or_else(
                            || "ERR".into(),
                            |t| format!("{}\t{}", t.seconds(), t.nanos()),
                        );
                        oracle.check(
                            &format!("Z\t{encoded}\t{}", hex(date.as_bytes())),
                            &observed,
                        );
                        dates += 1;
                    }
                }
            }
        }
    }
    oracle.finish();
    eprintln!(
        "Fixed Go TZif: {lookups} actual location lookups, {dates} original HTTP date parses; synthetic v1/v2/v3 sources, no daemon timezone loading."
    );
}
