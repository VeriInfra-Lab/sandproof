//! Explicit Linux amd64 errno, never a host-locale or arbitrary I/O message.
// Error table from Go 1.26.8 syscall/zerrors_linux_amd64.go.
// Copyright 2009 The Go Authors. BSD license: ../multipart/GO-LICENSE.
use std::{borrow::Cow, fmt};

/// Only construct from Linux native errno or explicit controlled fixtures.
/// The number has Linux amd64 semantics even when a portable test runs elsewhere.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LinuxErrno(pub i32);
impl LinuxErrno {
    #[must_use]
    pub fn message(self) -> Cow<'static, str> {
        usize::try_from(self.0)
            .ok()
            .and_then(|n| MESSAGES.get(n))
            .filter(|text| !text.is_empty())
            .map_or_else(
                || Cow::Owned(format!("errno {}", self.0)),
                |text| Cow::Borrowed(*text),
            )
    }
}
impl fmt::Display for LinuxErrno {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message())
    }
}
impl std::error::Error for LinuxErrno {}

const MESSAGES: &[&str] = &[
    "",                                                  // 0
    "operation not permitted",                           // 1
    "no such file or directory",                         // 2
    "no such process",                                   // 3
    "interrupted system call",                           // 4
    "input/output error",                                // 5
    "no such device or address",                         // 6
    "argument list too long",                            // 7
    "exec format error",                                 // 8
    "bad file descriptor",                               // 9
    "no child processes",                                // 10
    "resource temporarily unavailable",                  // 11
    "cannot allocate memory",                            // 12
    "permission denied",                                 // 13
    "bad address",                                       // 14
    "block device required",                             // 15
    "device or resource busy",                           // 16
    "file exists",                                       // 17
    "invalid cross-device link",                         // 18
    "no such device",                                    // 19
    "not a directory",                                   // 20
    "is a directory",                                    // 21
    "invalid argument",                                  // 22
    "too many open files in system",                     // 23
    "too many open files",                               // 24
    "inappropriate ioctl for device",                    // 25
    "text file busy",                                    // 26
    "file too large",                                    // 27
    "no space left on device",                           // 28
    "illegal seek",                                      // 29
    "read-only file system",                             // 30
    "too many links",                                    // 31
    "broken pipe",                                       // 32
    "numerical argument out of domain",                  // 33
    "numerical result out of range",                     // 34
    "resource deadlock avoided",                         // 35
    "file name too long",                                // 36
    "no locks available",                                // 37
    "function not implemented",                          // 38
    "directory not empty",                               // 39
    "too many levels of symbolic links",                 // 40
    "",                                                  // 41
    "no message of desired type",                        // 42
    "identifier removed",                                // 43
    "channel number out of range",                       // 44
    "level 2 not synchronized",                          // 45
    "level 3 halted",                                    // 46
    "level 3 reset",                                     // 47
    "link number out of range",                          // 48
    "protocol driver not attached",                      // 49
    "no CSI structure available",                        // 50
    "level 2 halted",                                    // 51
    "invalid exchange",                                  // 52
    "invalid request descriptor",                        // 53
    "exchange full",                                     // 54
    "no anode",                                          // 55
    "invalid request code",                              // 56
    "invalid slot",                                      // 57
    "",                                                  // 58
    "bad font file format",                              // 59
    "device not a stream",                               // 60
    "no data available",                                 // 61
    "timer expired",                                     // 62
    "out of streams resources",                          // 63
    "machine is not on the network",                     // 64
    "package not installed",                             // 65
    "object is remote",                                  // 66
    "link has been severed",                             // 67
    "advertise error",                                   // 68
    "srmount error",                                     // 69
    "communication error on send",                       // 70
    "protocol error",                                    // 71
    "multihop attempted",                                // 72
    "RFS specific error",                                // 73
    "bad message",                                       // 74
    "value too large for defined data type",             // 75
    "name not unique on network",                        // 76
    "file descriptor in bad state",                      // 77
    "remote address changed",                            // 78
    "can not access a needed shared library",            // 79
    "accessing a corrupted shared library",              // 80
    ".lib section in a.out corrupted",                   // 81
    "attempting to link in too many shared libraries",   // 82
    "cannot exec a shared library directly",             // 83
    "invalid or incomplete multibyte or wide character", // 84
    "interrupted system call should be restarted",       // 85
    "streams pipe error",                                // 86
    "too many users",                                    // 87
    "socket operation on non-socket",                    // 88
    "destination address required",                      // 89
    "message too long",                                  // 90
    "protocol wrong type for socket",                    // 91
    "protocol not available",                            // 92
    "protocol not supported",                            // 93
    "socket type not supported",                         // 94
    "operation not supported",                           // 95
    "protocol family not supported",                     // 96
    "address family not supported by protocol",          // 97
    "address already in use",                            // 98
    "cannot assign requested address",                   // 99
    "network is down",                                   // 100
    "network is unreachable",                            // 101
    "network dropped connection on reset",               // 102
    "software caused connection abort",                  // 103
    "connection reset by peer",                          // 104
    "no buffer space available",                         // 105
    "transport endpoint is already connected",           // 106
    "transport endpoint is not connected",               // 107
    "cannot send after transport endpoint shutdown",     // 108
    "too many references: cannot splice",                // 109
    "connection timed out",                              // 110
    "connection refused",                                // 111
    "host is down",                                      // 112
    "no route to host",                                  // 113
    "operation already in progress",                     // 114
    "operation now in progress",                         // 115
    "stale file handle",                                 // 116
    "structure needs cleaning",                          // 117
    "not a XENIX named type file",                       // 118
    "no XENIX semaphores available",                     // 119
    "is a named type file",                              // 120
    "remote I/O error",                                  // 121
    "disk quota exceeded",                               // 122
    "no medium found",                                   // 123
    "wrong medium type",                                 // 124
    "operation canceled",                                // 125
    "required key not available",                        // 126
    "key has expired",                                   // 127
    "key has been revoked",                              // 128
    "key was rejected by service",                       // 129
    "owner died",                                        // 130
    "state not recoverable",                             // 131
    "operation not possible due to RF-kill",             // 132
];
