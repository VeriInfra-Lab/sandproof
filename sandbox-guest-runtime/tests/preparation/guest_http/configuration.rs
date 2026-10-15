use super::*;
use sandproof_guest_runtime::{
    file_mime::{Catalog, CatalogLimits},
    filesystem::{FileLimits, transfer::TransferLimits},
};
use sandproof_guest_service::{
    file_http::Settings as HttpSettings,
    file_transfer::{
        BatchLimits, DownloadLimits, GzipLimits, HttpBodyLimits, MultipartLimits, UploadLimits,
    },
    files::{FileContextLimits, FileSettings},
};
use std::sync::Arc;

pub(super) struct Pools {
    pub ordinary: CommandRequests,
    pub control: CommandRequests,
    pub files: FileRequests,
    pub rpc: FileRpcRequests,
    pub streams: FileStreams,
}
pub(super) struct Parts<M> {
    pub components: Components<M, CurrentAccount, Utc>,
    pub pools: Pools,
}
pub(super) fn parts<M: MetadataSource>(
    fixture: &Fixture,
    registry: &PidRegistry,
    session: &Session<M>,
) -> Parts<M> {
    let ordinary = requests(1);
    let control = requests(1);
    let commands = CommandHttp::new(
        session.clone(),
        RequestPools {
            ordinary: ordinary.clone(),
            control: control.clone(),
        },
        accounts(fixture),
        settings(),
        processes(fixture, registry, &PidRegistrar::new(store(fixture))),
        responses(),
    )
    .unwrap();
    let files = FileRequests::new(sandproof_guest_service::file_requests::Limits {
        in_flight: 1,
        timeout: Duration::from_secs(30),
        query: sandproof_guest_service::file_query::QueryLimits {
            raw_bytes: 4096,
            fields: 16,
        },
        authorization: sandproof_guest_service::file_authorization::Limits {
            headers: sandproof_guest_service::file_upload::HeaderLimits {
                bytes: 4096,
                fields: 16,
            },
            query_bytes: 4096,
        },
        rejection: sandproof_guest_service::upload_response::Limits {
            wire_bytes: 8192,
            chunk_bytes: 8192,
        },
    })
    .unwrap();
    let rpc = FileRpcRequests::new(sandproof_guest_service::file_rpc_requests::Limits {
        in_flight: 1,
        timeout: Duration::from_secs(30),
        headers: sandproof_guest_service::file_upload::HeaderLimits {
            bytes: 4096,
            fields: 16,
        },
        body: HttpBodyLimits {
            total_bytes: 8192,
            frame_bytes: 8192,
            frames: 64,
            trailer_bytes: 1024,
            trailer_fields: 16,
        },
    })
    .unwrap();
    let streams = FileStreams::new(
        1,
        TransferLimits {
            file: file_limits(),
            chunk_bytes: 4096,
            total_bytes: 8192,
        },
        Duration::from_secs(30),
    )
    .unwrap();
    let file_settings = http_settings(fixture.directory.path());
    let filesystem = FileRpcHttp::new(
        session.clone(),
        rpc.clone(),
        sandproof_guest_service::file_io::FilePool::new(1, file_limits(), Duration::from_secs(30))
            .unwrap(),
        accounts(fixture),
        file_settings.files.clone(),
        sandproof_guest_service::file_rpc_response::Limits {
            wire_bytes: 8192,
            chunk_bytes: 8192,
        },
    )
    .unwrap();
    let file_http = FileHttp::new(
        session.clone(),
        files.clone(),
        streams.clone(),
        accounts(fixture),
        file_settings,
    )
    .unwrap();
    Parts {
        components: Components {
            commands,
            files: file_http,
            filesystem,
        },
        pools: Pools {
            ordinary,
            control,
            files,
            rpc,
            streams,
        },
    }
}
fn file_limits() -> FileLimits {
    FileLimits {
        path_bytes: 4096,
        entries: 16,
        retained_bytes: 8192,
        visited: 64,
        xattr_bytes: 8192,
        account_bytes: 8192,
    }
}
fn http_settings(root: &Path) -> HttpSettings<Utc> {
    let http_limits = HttpBodyLimits {
        total_bytes: 8192,
        frame_bytes: 8192,
        frames: 64,
        trailer_bytes: 1024,
        trailer_fields: 16,
    };
    HttpSettings {
        files: FileSettings {
            daemon_workdir: root.to_str().unwrap().into(),
            limits: FileContextLimits {
                path_bytes: 4096,
                username_bytes: 64,
                authorization_bytes: 1024,
            },
        },
        upload: UploadLimits {
            headers: sandproof_guest_service::file_upload::HeaderLimits {
                bytes: 4096,
                fields: 16,
            },
            encoding: sandproof_guest_service::file_encoding::Limits {
                header_bytes: 4096,
                items: 16,
            },
            gzip: GzipLimits {
                http: http_limits,
                decode: sandproof_guest_runtime::gzip::DecodeLimits {
                    chunk_bytes: 4096,
                    encoded_bytes: 8192,
                    decoded_bytes: 8192,
                    header_bytes: 4096,
                    members: 16,
                },
            },
            multipart: MultipartLimits {
                wire: sandproof_guest_runtime::multipart::Limits {
                    chunk_bytes: 4096,
                    boundary_bytes: 256,
                    header_bytes: 4096,
                    header_count: 16,
                    line_bytes: 4096,
                    parts: 16,
                    total_bytes: 8192,
                },
                frame_bytes: 8192,
                frames: 64,
                trailer_bytes: 1024,
                trailer_fields: 16,
            },
            batch: BatchLimits {
                files: 16,
                total_bytes: 8192,
                retained_bytes: 8192,
            },
        },
        download: DownloadLimits {
            response: sandproof_guest_service::file_download::Limits {
                request: sandproof_guest_runtime::file_download::Limits {
                    header_bytes: 4096,
                    items: 16,
                },
                response_header_bytes: 4096,
                wire_bytes: 8192,
            },
            encoding: sandproof_guest_service::file_encoding::Limits {
                header_bytes: 4096,
                items: 16,
            },
            compressed_bytes: 8192,
        },
        upload_response: sandproof_guest_service::upload_response::Limits {
            wire_bytes: 8192,
            chunk_bytes: 4096,
        },
        mime: Arc::new(
            Catalog::from_sources(
                [None; 2],
                [None; 4],
                CatalogLimits {
                    input_bytes: 65536,
                    line_bytes: 4096,
                    entries: 1024,
                    retained_bytes: 65536,
                    value_bytes: 4096,
                },
            )
            .unwrap(),
        ),
        local: Arc::new(Utc),
    }
}
