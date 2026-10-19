use super::{Catalog, CatalogLimits, Error};
use std::{fs::File, io::Read};

impl Catalog {
    /// Read the same ordered Linux MIME locations as fixed Go. Invoke only in
    /// admitted blocking startup work; this does not start/refresh a global
    /// table or interrupt an active filesystem call. Template evidence must
    /// bind these source materials, not assume host and guest tables match.
    /// # Errors
    /// Budget/configuration, malformed text or read failure. Open failures are
    /// skipped as in Go; a successfully opened empty globs2 still wins.
    pub fn load_system(limits: CatalogLimits) -> Result<Self, Error> {
        load_paths(
            [
                "/usr/local/share/mime/globs2",
                "/usr/share/mime/globs2",
                "/etc/mime.types",
                "/etc/apache2/mime.types",
                "/etc/apache/mime.types",
                "/etc/httpd/conf/mime.types",
            ],
            limits,
        )
    }
}
fn load_paths(paths: [&str; 6], limits: CatalogLimits) -> Result<Catalog, Error> {
    // Validate before any I/O, including the required builtin storage budget.
    let _ = Catalog::from_sources([None; 2], [None; 4], limits)?;
    let mut bytes: [Option<Vec<u8>>; 6] = std::array::from_fn(|_| None);
    let mut remaining = limits.input_bytes;
    for (index, path) in paths.into_iter().enumerate() {
        let Ok(file) = File::open(path) else {
            continue;
        };
        let mut data = Vec::new();
        file.take((remaining as u64) + 1)
            .read_to_end(&mut data)
            .map_err(|_| Error::Io)?;
        remaining = remaining.checked_sub(data.len()).ok_or(Error::InputLimit)?;
        bytes[index] = Some(data);
        if index < 2 {
            break;
        }
    }
    Catalog::from_sources(
        [bytes[0].as_deref(), bytes[1].as_deref()],
        [
            bytes[2].as_deref(),
            bytes[3].as_deref(),
            bytes[4].as_deref(),
            bytes[5].as_deref(),
        ],
        limits,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fixed_order_empty_globs_stops_fallback_and_aggregate_reads_are_bounded() {
        let root = tempfile::tempdir().unwrap();
        let names: Vec<_> = (0..6).map(|n| root.path().join(n.to_string())).collect();
        let paths = std::array::from_fn(|n| names[n].to_str().unwrap());
        let limits = super::super::tests::limits();
        std::fs::write(&names[2], b"application/custom txt").unwrap();
        let types = load_paths(paths, limits).unwrap();
        assert_eq!(types.lookup(".txt").unwrap(), Some("application/custom"));
        std::fs::write(&names[1], b"").unwrap();
        let types = load_paths(paths, limits).unwrap();
        assert_eq!(
            types.lookup(".txt").unwrap(),
            Some("text/plain; charset=utf-8")
        );
        std::fs::write(&names[0], b"12345").unwrap();
        assert!(matches!(
            load_paths(
                paths,
                CatalogLimits {
                    input_bytes: 4,
                    ..limits
                }
            ),
            Err(Error::InputLimit)
        ));
        std::fs::remove_file(&names[0]).unwrap();
        std::fs::remove_file(&names[1]).unwrap();
        std::fs::create_dir(&names[0]).unwrap();
        assert!(matches!(load_paths(paths, limits), Err(Error::Io)));
    }
}
