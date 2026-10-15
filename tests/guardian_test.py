"""Subprocess acceptance of the parent-lifetime pipe; never changes networking."""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time
import unittest

BINARY = Path(os.environ.get('XPROXY_BINARY', str(Path(__file__).resolve().parents[1] / 'src-tauri/target/debug' / ('xproxy-desktop.exe' if os.name == 'nt' else 'xproxy-desktop'))))

def alive(pid):
    if os.name == 'nt':
        import ctypes
        api = ctypes.WinDLL('kernel32', use_last_error=True)
        api.OpenProcess.restype = ctypes.c_void_p
        api.OpenProcess.argtypes = [ctypes.c_uint32, ctypes.c_int, ctypes.c_uint32]
        api.GetExitCodeProcess.argtypes = [ctypes.c_void_p, ctypes.POINTER(ctypes.c_uint32)]
        api.CloseHandle.argtypes = [ctypes.c_void_p]
        handle = api.OpenProcess(0x1000, False, pid)
        if not handle:
            return False
        code = ctypes.c_uint32()
        try:
            return bool(api.GetExitCodeProcess(handle, ctypes.byref(code))) and code.value == 259
        finally:
            api.CloseHandle(handle)
    try:
        os.kill(pid, 0)
        return True
    except ProcessLookupError:
        return False

def wait_file(path):
    deadline = time.monotonic() + 4
    while not path.exists() and time.monotonic() < deadline:
        time.sleep(.025)
    if not path.exists():
        raise AssertionError('guardian child failed to start')
    return int(path.read_text())

def wait_dead(pid):
    deadline = time.monotonic() + 4
    while alive(pid) and time.monotonic() < deadline:
        time.sleep(.025)
    if alive(pid):
        raise AssertionError('SSH substitute survived parent lease loss')

class GuardianTest(unittest.TestCase):
    def test_pipe_eof_reaps_child(self):
        with tempfile.TemporaryDirectory(prefix='xproxy-guardian-') as directory:
            pidfile = Path(directory) / 'pid'
            code = f'import os,time,pathlib; pathlib.Path({str(pidfile)!r}).write_text(str(os.getpid())); time.sleep(60)'
            process = subprocess.Popen([str(BINARY), '--ssh-guardian', sys.executable, '-c', code], stdin=subprocess.PIPE, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
            try:
                pid = wait_file(pidfile)
                process.stdin.close()
                self.assertEqual(process.wait(timeout=4), 0)
                wait_dead(pid)
            finally:
                if not process.stdin.closed:
                    process.stdin.close()
                if process.poll() is None:
                    process.wait(timeout=4)
                process.stderr.close()

    def test_abrupt_parent_exit_reaps_child(self):
        with tempfile.TemporaryDirectory(prefix='xproxy-crash-') as directory:
            pidfile = Path(directory) / 'pid'
            code = f'import os,time,pathlib; pathlib.Path({str(pidfile)!r}).write_text(str(os.getpid())); time.sleep(60)'
            parent = '\n'.join([
                'import subprocess,sys,time,os,pathlib',
                f'p=subprocess.Popen({[str(BINARY), "--ssh-guardian", sys.executable, "-c", code]!r},stdin=subprocess.PIPE,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)',
                f'path=pathlib.Path({str(pidfile)!r})',
                'deadline=time.monotonic()+4',
                'while not path.exists() and time.monotonic()<deadline: time.sleep(.025)',
                'os._exit(99 if path.exists() else 2)',
            ])
            result = subprocess.run([sys.executable, '-c', parent], timeout=6)
            self.assertEqual(result.returncode, 99)
            wait_dead(wait_file(pidfile))

if __name__ == '__main__':
    unittest.main()
