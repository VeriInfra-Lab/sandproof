"""Command/file samples shared by synthetic, Rust-gateway and real-VM checks.

Only inputs and assertions live here. Synthetic transports supply these bytes;
the real probe executes the command and reads actual files independently.
"""

from io import BytesIO

from e2b.sandbox.filesystem.filesystem import FileType


FILES = {"text.txt": "spaces and 中文\n".encode(),
         "nested/binary.bin": bytes(range(256)) * 129, "nested/deep/empty": b""}
COMMAND = "pwd; printf '%s\\n' \"$SP_DATA_VALUE\"; id -un; printf 'data-stderr\\n' >&2"
ENV = {"SP_DATA_VALUE": "spaces and 中文"}
NONZERO_COMMAND = "printf failed >&2; exit 17"
NONZERO_RESULT = (17, "failed")


def command_result(directory, user):
    return (f"{directory}\n{ENV['SP_DATA_VALUE']}\n{user}\n", "data-stderr\n", 0)


def validate_command(result, directory, user):
    actual = (result.stdout, result.stderr, result.exit_code)
    expected = command_result(directory, user)
    assert actual == expected, {"user": user, "actual": actual, "expected": expected}


def validate_nonzero(error):
    assert (error.exit_code, error.stderr) == NONZERO_RESULT


def file_entries(directory):
    # Return fresh streams for each client/user/profile; consumed cursors must
    # never make a later upload accidentally test an empty file.
    return [{"path": f"{directory}/{name}", "data": BytesIO(data)}
            for name, data in FILES.items()]


def validate_entries(entries, directory):
    files = {entry.path: entry.size for entry in entries if entry.type == FileType.FILE}
    assert files == {f"{directory}/{name}": len(data) for name, data in FILES.items()}, files


def validate_file_info(info, path, content):
    assert (info.path, info.size, info.type) == (path, len(content), FileType.FILE)


def validate_content(content, expected):
    assert content == expected
