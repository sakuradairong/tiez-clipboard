#!/usr/bin/env python3
"""Measure a GUI process tree on Linux.

Run this script inside an X server, for example:

    xvfb-run -a python3 measure_linux.py --label slint --runs 5 \
      --window-title "TieZ native UI feasibility PoC" -- \
      ./target/release/tiez-native-ui-poc

The script reports aggregate RSS, PSS, and private memory (USS approximation)
for the root process and all descendants. It is intended for same-host screening,
not as a replacement for Windows WebView2 measurements.
"""

from __future__ import annotations

import argparse
import ctypes
import ctypes.util
import json
import os
import platform
import re
import signal
import shutil
import statistics
import subprocess
import sys
import tempfile
import time
from pathlib import Path
from typing import Any


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--label", required=True)
    parser.add_argument("--runs", type=int, default=5)
    parser.add_argument("--window-title", required=True)
    parser.add_argument("--warmup-seconds", type=float, default=2.0)
    parser.add_argument("--window-timeout-seconds", type=float, default=15.0)
    parser.add_argument(
        "--sample-state",
        choices=("visible", "hidden", "destroyed"),
        default="visible",
        help="window state that must be reached before the warmup and samples",
    )
    parser.add_argument("--state-timeout-seconds", type=float, default=15.0)
    parser.add_argument("--samples-per-run", type=int, default=3)
    parser.add_argument("--sample-interval-seconds", type=float, default=0.25)
    parser.add_argument(
        "--state-trigger-key",
        help="X11 key to send to the measured window before waiting for the sample state",
    )
    parser.add_argument("--state-trigger-count", type=int, default=1)
    parser.add_argument("--state-trigger-delay-seconds", type=float, default=0.0)
    parser.add_argument("--state-trigger-interval-seconds", type=float, default=0.1)
    parser.add_argument("--output")
    parser.add_argument("command", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    if args.command and args.command[0] == "--":
        args.command = args.command[1:]
    if not args.command:
        parser.error("a command is required after --")
    if args.runs < 1:
        parser.error("--runs must be at least 1")
    if args.samples_per_run < 1:
        parser.error("--samples-per-run must be at least 1")
    if args.state_trigger_count < 1:
        parser.error("--state-trigger-count must be at least 1")
    for name in (
        "warmup_seconds",
        "window_timeout_seconds",
        "state_timeout_seconds",
        "sample_interval_seconds",
        "state_trigger_delay_seconds",
        "state_trigger_interval_seconds",
    ):
        if getattr(args, name) < 0:
            parser.error(f"--{name.replace('_', '-')} must not be negative")
    return args


def read_ppids() -> dict[int, int]:
    result: dict[int, int] = {}
    for entry in Path("/proc").iterdir():
        if not entry.name.isdigit():
            continue
        try:
            # Field 2 is wrapped in parentheses and may contain spaces. The
            # parent PID is field 4, immediately after the final ") <state> ".
            stat = (entry / "stat").read_text()
            suffix = stat[stat.rfind(")") + 2 :].split()
            result[int(entry.name)] = int(suffix[1])
        except (FileNotFoundError, PermissionError, ValueError, IndexError):
            continue
    return result


def process_tree(root_pid: int) -> list[int]:
    ppids = read_ppids()
    found = {root_pid}
    changed = True
    while changed:
        changed = False
        for pid, ppid in ppids.items():
            if ppid in found and pid not in found:
                found.add(pid)
                changed = True
    return sorted(found)


def read_process_name(pid: int) -> str:
    try:
        return Path(f"/proc/{pid}/comm").read_text().strip()
    except (FileNotFoundError, PermissionError):
        return "<exited>"


def read_memory_kib(pid: int) -> tuple[int, int, int]:
    values: dict[str, int] = {}
    try:
        for line in Path(f"/proc/{pid}/smaps_rollup").read_text().splitlines():
            key, _, rest = line.partition(":")
            if key in {"Rss", "Pss", "Private_Clean", "Private_Dirty"}:
                values[key] = int(rest.strip().split()[0])
    except (FileNotFoundError, PermissionError, ValueError):
        return 0, 0, 0
    private = values.get("Private_Clean", 0) + values.get("Private_Dirty", 0)
    return values.get("Rss", 0), values.get("Pss", 0), private


def aggregate_memory(root_pid: int) -> dict[str, Any]:
    details = []
    rss = pss = private = 0
    for pid in process_tree(root_pid):
        process_rss, process_pss, process_private = read_memory_kib(pid)
        if process_rss == process_pss == process_private == 0:
            continue
        rss += process_rss
        pss += process_pss
        private += process_private
        details.append(
            {
                "pid": pid,
                "name": read_process_name(pid),
                "rss_kib": process_rss,
                "pss_kib": process_pss,
                "private_kib": process_private,
            }
        )
    return {
        "process_count": len(details),
        "rss_kib": rss,
        "pss_kib": pss,
        "private_kib": private,
        "processes": details,
    }


def x11_window_ids() -> list[str]:
    try:
        result = subprocess.run(
            ["xwininfo", "-root", "-tree"],
            check=False,
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
            text=True,
            timeout=2,
        )
    except (FileNotFoundError, subprocess.TimeoutExpired):
        return []
    if result.returncode != 0:
        return []
    return re.findall(r"^\s*(0x[0-9a-fA-F]+)\s+", result.stdout, re.MULTILINE)


def x11_window_identity(window_id: str) -> tuple[int | None, str | None]:
    try:
        result = subprocess.run(
            ["xprop", "-id", window_id, "_NET_WM_PID", "_NET_WM_NAME", "WM_NAME"],
            check=False,
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
            text=True,
            timeout=2,
        )
    except (FileNotFoundError, subprocess.TimeoutExpired):
        return None, None
    if result.returncode != 0:
        return None, None

    pid_match = re.search(r"_NET_WM_PID\([^)]*\)\s*=\s*(\d+)", result.stdout)
    pid = int(pid_match.group(1)) if pid_match else None
    title = None
    for key in ("_NET_WM_NAME", "WM_NAME"):
        match = re.search(rf"^{key}\([^)]*\)\s*=\s*\"(.*)\"$", result.stdout, re.MULTILINE)
        if match:
            title = match.group(1)
            break
    return pid, title


def x11_window_state(window_id: str) -> str:
    try:
        result = subprocess.run(
            ["xwininfo", "-id", window_id],
            check=False,
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
            text=True,
            timeout=2,
        )
    except (FileNotFoundError, subprocess.TimeoutExpired):
        return "absent"
    if result.returncode != 0:
        return "absent"
    match = re.search(r"^\s*Map State:\s*(\S+)", result.stdout, re.MULTILINE)
    if match and match.group(1) == "IsViewable":
        return "visible"
    return "hidden"


def find_process_window(
    root_pid: int, title: str, state: str | None = "visible"
) -> dict[str, Any] | None:
    process_ids = set(process_tree(root_pid))
    for window_id in x11_window_ids():
        pid, actual_title = x11_window_identity(window_id)
        if pid not in process_ids or actual_title != title:
            continue
        actual_state = x11_window_state(window_id)
        if state is None or actual_state == state:
            return {"id": window_id, "pid": pid, "title": actual_title, "state": actual_state}
    return None


def target_state_reached(root_pid: int, window_id: str, title: str, target: str) -> bool:
    state = x11_window_state(window_id)
    if target == "destroyed":
        return state == "absent" and find_process_window(root_pid, title, None) is None
    return state == target


def send_x11_key(window_id: str, key: str, count: int, interval_seconds: float) -> None:
    x11_path = ctypes.util.find_library("X11")
    xtst_path = ctypes.util.find_library("Xtst")
    if not x11_path or not xtst_path:
        raise RuntimeError("X11 key injection requires libX11 and libXtst")

    x11 = ctypes.CDLL(x11_path)
    xtst = ctypes.CDLL(xtst_path)
    x11.XOpenDisplay.argtypes = [ctypes.c_char_p]
    x11.XOpenDisplay.restype = ctypes.c_void_p
    x11.XCloseDisplay.argtypes = [ctypes.c_void_p]
    x11.XStringToKeysym.argtypes = [ctypes.c_char_p]
    x11.XStringToKeysym.restype = ctypes.c_ulong
    x11.XKeysymToKeycode.argtypes = [ctypes.c_void_p, ctypes.c_ulong]
    x11.XKeysymToKeycode.restype = ctypes.c_ubyte
    x11.XSetInputFocus.argtypes = [ctypes.c_void_p, ctypes.c_ulong, ctypes.c_int, ctypes.c_ulong]
    x11.XFlush.argtypes = [ctypes.c_void_p]
    xtst.XTestFakeKeyEvent.argtypes = [ctypes.c_void_p, ctypes.c_uint, ctypes.c_int, ctypes.c_ulong]

    display = x11.XOpenDisplay(None)
    if not display:
        raise RuntimeError("could not open DISPLAY for X11 key injection")
    try:
        key_names = [part.strip() for part in key.split("+") if part.strip()]
        if not key_names:
            raise RuntimeError("X11 key combination must not be empty")
        keycodes = []
        aliases = {"Alt": "Alt_L", "Control": "Control_L", "Ctrl": "Control_L", "Shift": "Shift_L"}
        for key_name in key_names:
            keysym = x11.XStringToKeysym(aliases.get(key_name, key_name).encode())
            keycode = x11.XKeysymToKeycode(display, keysym) if keysym else 0
            if not keycode:
                raise RuntimeError(f"X11 could not resolve key {key_name!r}")
            keycodes.append(keycode)

        x11.XSetInputFocus(display, int(window_id, 16), 2, 0)
        x11.XFlush(display)
        for index in range(count):
            for keycode in keycodes:
                xtst.XTestFakeKeyEvent(display, keycode, 1, 0)
            for keycode in reversed(keycodes):
                xtst.XTestFakeKeyEvent(display, keycode, 0, 0)
            x11.XFlush(display)
            if index + 1 < count:
                time.sleep(interval_seconds)
    finally:
        x11.XCloseDisplay(display)


def stop_process(process: subprocess.Popen[str]) -> None:
    if process.poll() is not None:
        return
    try:
        os.killpg(process.pid, signal.SIGTERM)
        process.wait(timeout=5)
    except (ProcessLookupError, subprocess.TimeoutExpired):
        if process.poll() is None:
            os.killpg(process.pid, signal.SIGKILL)
            process.wait(timeout=5)


def read_captured_output(stream: Any) -> str:
    stream.flush()
    stream.seek(0)
    return stream.read()


def sample_process_tree(
    root_pid: int, count: int, interval_seconds: float
) -> tuple[dict[str, Any], list[dict[str, Any]]]:
    samples = []
    for sample_index in range(count):
        sample = aggregate_memory(root_pid)
        if sample["process_count"] == 0:
            raise RuntimeError("no process memory was readable")
        samples.append(sample)
        if sample_index + 1 < count:
            time.sleep(interval_seconds)

    keys = ["process_count", "rss_kib", "pss_kib", "private_kib"]
    medians = {key: statistics.median(sample[key] for sample in samples) for key in keys}
    representative = min(samples, key=lambda sample: abs(sample["private_kib"] - medians["private_kib"]))
    metrics = {**medians, "processes": representative["processes"]}
    return metrics, samples


def run_once(args: argparse.Namespace, index: int) -> dict[str, Any]:
    start = time.monotonic()
    stdout = tempfile.TemporaryFile(mode="w+t")
    stderr = tempfile.TemporaryFile(mode="w+t")
    process = subprocess.Popen(args.command, stdout=stdout, stderr=stderr, text=True, start_new_session=True)
    try:
        deadline = start + args.window_timeout_seconds
        window = None
        while time.monotonic() < deadline:
            if process.poll() is not None:
                raise RuntimeError(
                    f"run {index}: process exited before its window appeared "
                    f"(code {process.returncode})\nstdout:\n{read_captured_output(stdout)}"
                    f"\nstderr:\n{read_captured_output(stderr)}"
                )
            window = find_process_window(process.pid, args.window_title)
            if window is not None:
                break
            time.sleep(0.025)
        else:
            raise RuntimeError(
                f"run {index}: a visible window titled {args.window_title!r} "
                "owned by the launched process tree was not observed"
            )

        window_ms = round((time.monotonic() - start) * 1000, 1)
        state_ms = window_ms
        if args.sample_state != "visible":
            if args.state_trigger_key:
                time.sleep(args.state_trigger_delay_seconds)
                send_x11_key(
                    window["id"],
                    args.state_trigger_key,
                    args.state_trigger_count,
                    args.state_trigger_interval_seconds,
                )
            deadline = time.monotonic() + args.state_timeout_seconds
            while time.monotonic() < deadline:
                if process.poll() is not None:
                    raise RuntimeError(
                        f"run {index}: process exited before reaching {args.sample_state!r}"
                    )
                if target_state_reached(
                    process.pid, window["id"], args.window_title, args.sample_state
                ):
                    state_ms = round((time.monotonic() - start) * 1000, 1)
                    break
                time.sleep(0.025)
            else:
                current_state = x11_window_state(window["id"])
                raise RuntimeError(
                    f"run {index}: window did not reach {args.sample_state!r}; "
                    f"last state was {current_state!r}"
                )

        time.sleep(args.warmup_seconds)
        if not target_state_reached(
            process.pid, window["id"], args.window_title, args.sample_state
        ):
            raise RuntimeError(f"run {index}: window left {args.sample_state!r} before sampling")
        metrics, samples = sample_process_tree(
            process.pid, args.samples_per_run, args.sample_interval_seconds
        )
        metrics.update(
            {
                "run": index,
                "window_ms": window_ms,
                "state_ms": state_ms,
                "window_id": window["id"],
                "window_pid": window["pid"],
                "samples": samples,
            }
        )
        return metrics
    finally:
        stop_process(process)
        stdout.close()
        stderr.close()


def median_summary(runs: list[dict[str, Any]]) -> dict[str, Any]:
    keys = ["window_ms", "state_ms", "process_count", "rss_kib", "pss_kib", "private_kib"]
    return {f"median_{key}": statistics.median(run[key] for run in runs) for key in keys}


def main() -> int:
    args = parse_args()
    missing_tools = [tool for tool in ("xwininfo", "xprop") if shutil.which(tool) is None]
    if missing_tools:
        raise RuntimeError(f"missing required X11 tools: {', '.join(missing_tools)}")

    runs = []
    for index in range(1, args.runs + 1):
        run = run_once(args, index)
        runs.append(run)
        print(
            f"JCODE_PROGRESS {json.dumps({'current': index, 'total': args.runs, 'unit': 'runs', 'message': args.label})}",
            flush=True,
        )

    report = {
        "label": args.label,
        "command": args.command,
        "display": os.environ.get("DISPLAY"),
        "host": {
            "platform": platform.platform(),
            "python": platform.python_version(),
        },
        "sample_state": args.sample_state,
        "warmup_seconds": args.warmup_seconds,
        "samples_per_run": args.samples_per_run,
        "sample_interval_seconds": args.sample_interval_seconds,
        "state_trigger": {
            "key": args.state_trigger_key,
            "count": args.state_trigger_count if args.state_trigger_key else 0,
            "delay_seconds": args.state_trigger_delay_seconds,
            "interval_seconds": args.state_trigger_interval_seconds,
        },
        "runs": runs,
        "summary": median_summary(runs),
    }
    rendered = json.dumps(report, indent=2, ensure_ascii=False)
    if args.output:
        Path(args.output).write_text(rendered + "\n")
    print(rendered)
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except Exception as error:
        print(f"benchmark failed: {error}", file=sys.stderr)
        raise SystemExit(1)
