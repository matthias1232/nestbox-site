"""Tests for the pure parts of the SSH door: stream framing and the command line.

`app.ssh_gateway` imports asyncssh lazily, so these run with the standard
library alone, in a container that never had a Docker socket.
"""
from __future__ import annotations

import struct
import unittest

from app import ssh_gateway


def frame(stream: int, payload: bytes) -> bytes:
    return bytes([stream, 0, 0, 0]) + struct.pack(">I", len(payload)) + payload


class DemuxTest(unittest.TestCase):
    def test_splits_stdout_and_stderr(self) -> None:
        data = frame(1, b"hello\n") + frame(2, b"oops\n")
        frames, rest = ssh_gateway.demux(data)
        self.assertEqual(frames, [(1, b"hello\n"), (2, b"oops\n")])
        self.assertEqual(rest, b"")

    def test_keeps_a_frame_that_arrived_in_two_reads(self) -> None:
        data = frame(1, b"abcdef")
        first, rest = ssh_gateway.demux(data[:10])
        self.assertEqual(first, [])
        again, rest = ssh_gateway.demux(rest + data[10:])
        self.assertEqual(again, [(1, b"abcdef")])
        self.assertEqual(rest, b"")

    def test_a_partial_header_is_kept_not_dropped(self) -> None:
        frames, rest = ssh_gateway.demux(b"\x01\x00\x00")
        self.assertEqual(frames, [])
        self.assertEqual(rest, b"\x01\x00\x00")

    def test_an_empty_frame_is_a_frame(self) -> None:
        frames, rest = ssh_gateway.demux(frame(1, b""))
        self.assertEqual(frames, [(1, b"")])
        self.assertEqual(rest, b"")


class ShellCommandTest(unittest.TestCase):
    def test_no_command_is_a_login_shell(self) -> None:
        self.assertEqual(ssh_gateway.shell_command(None), ["bash", "-l"])
        self.assertEqual(ssh_gateway.shell_command(""), ["bash", "-l"])

    def test_a_command_is_one_argument_never_spliced(self) -> None:
        command = 'claude --model "x"; echo $HOME'
        self.assertEqual(ssh_gateway.shell_command(command), ["bash", "-lc", command])


class TmuxTabTest(unittest.TestCase):
    def test_a_plain_login_means_the_shared_ssh_tab(self) -> None:
        self.assertEqual(ssh_gateway.tmux_tab(None), "ssh")
        self.assertEqual(ssh_gateway.tmux_tab(""), "ssh")
        self.assertEqual(ssh_gateway.tmux_tab("   "), "ssh")

    def test_attach_and_tab_name_the_tab_to_join(self) -> None:
        self.assertEqual(ssh_gateway.tmux_tab("attach t-1"), "t-1")
        self.assertEqual(ssh_gateway.tmux_tab("tab t_1"), "t_1")

    def test_anything_else_is_a_command_not_a_tab(self) -> None:
        for command in ("tab x y", "ls -la", "attach ../x", "attach " + "x" * 65, "attach x; rm -rf /"):
            with self.subTest(command=command):
                self.assertIsNone(ssh_gateway.tmux_tab(command))


class ShellCommandWithTmuxTest(unittest.TestCase):
    def test_a_login_lands_in_the_shared_ssh_tab(self) -> None:
        self.assertEqual(
            ssh_gateway.shell_command(None, has_tmux=True),
            ["tmux", "new-session", "-A", "-s", "nestbox-ssh"],
        )

    def test_attach_joins_the_named_tab(self) -> None:
        self.assertEqual(
            ssh_gateway.shell_command("attach t-9", has_tmux=True),
            ["tmux", "new-session", "-A", "-s", "nestbox-t-9"],
        )

    def test_a_command_stays_a_command_even_with_tmux(self) -> None:
        self.assertEqual(ssh_gateway.shell_command("ls -la", has_tmux=True), ["bash", "-lc", "ls -la"])


class HostKeyPathTest(unittest.TestCase):
    def test_the_key_lives_in_the_state_directory(self) -> None:
        self.assertEqual(str(ssh_gateway.host_key_path("/var/lib/x")), "/var/lib/x/ssh_host_ed25519_key")


if __name__ == "__main__":
    unittest.main()


class PasswordTest(unittest.TestCase):
    def entry(self, user="tmp-1", salt="s", password="pw", expires=2_000_000_000):
        import hashlib

        return {"username": user, "salt": salt, "hash": hashlib.sha256(f"{salt}{password}".encode()).hexdigest(), "expires_at": expires}

    def test_the_right_user_and_password_within_the_time(self) -> None:
        self.assertTrue(ssh_gateway.password_matches([self.entry()], "tmp-1", "pw", now=1.0))

    def test_a_wrong_password_a_wrong_user_and_an_expired_login_are_refused(self) -> None:
        entries = [self.entry()]
        self.assertFalse(ssh_gateway.password_matches(entries, "tmp-1", "nope", now=1.0))
        self.assertFalse(ssh_gateway.password_matches(entries, "root", "pw", now=1.0))
        self.assertFalse(ssh_gateway.password_matches(entries, "tmp-1", "pw", now=2_000_000_001))

    def test_no_logins_means_no_password_works(self) -> None:
        self.assertFalse(ssh_gateway.password_matches([], "tmp-1", "pw", now=1.0))


class ClientVanishesTests(unittest.IsolatedAsyncioTestCase):
    async def test_a_terminal_session_ends_when_the_client_disappears(self) -> None:
        import asyncio
        import threading

        from app import ssh_gateway

        release = threading.Event()

        class Sock:
            written: list[bytes] = []

            def read(self, _n: int) -> bytes:
                release.wait(5)  # the shell is idle: nothing arrives until it ends
                return b""

            def write(self, data: bytes) -> None:
                self.written.append(data)
                release.set()  # Ctrl-D ends the shell

        class Run:
            tty = True
            tab = None
            gateway = ""
            bytes_in = bytes_out = 0
            sock = Sock()

            def end_input(self) -> None:
                pass  # a terminal session has no end-of-file

            def close(self) -> None:
                release.set()

        class Stdin:
            async def read(self, _n: int) -> bytes:
                return b""  # the client is gone

        class Process:
            stdin = Stdin()

        await asyncio.wait_for(ssh_gateway._pump(Process(), Run()), 3)
        self.assertIn(b"\x04\x04", Sock.written, "the idle shell is told to end instead of leaking")

    async def test_a_tmux_session_is_only_detached(self) -> None:
        import asyncio
        import threading

        from app import ssh_gateway

        release = threading.Event()

        class Sock:
            written: list[bytes] = []

            def read(self, _n: int) -> bytes:
                release.wait(5)
                return b""

            def write(self, data: bytes) -> None:
                self.written.append(data)

        class Run:
            tty = True
            tab = "ssh"
            gateway = ""
            bytes_in = bytes_out = 0
            sock = Sock()

            def end_input(self) -> None:
                pass

            def close(self) -> None:
                release.set()  # closing the door releases the reader

        class Stdin:
            async def read(self, _n: int) -> bytes:
                return b""

        class Process:
            stdin = Stdin()

        await asyncio.wait_for(ssh_gateway._pump(Process(), Run()), 3)
        self.assertEqual(Sock.written, [], "a tab outlives the connection; nothing is typed into it")
