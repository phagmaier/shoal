#!/usr/bin/env python3
"""T00-only measurement driver. Stdlib; runs existing release binaries, never builds.

python3 spike/measure.py frames   [--out DIR] [--bin PATH] [--frames N] [--verify]
python3 spike/measure.py cat-idle [--out DIR] [--bin PATH] [--verify]
python3 spike/measure.py summary  [--out DIR]
Requires the measured Hyprland 0.56 Lua dispatcher API. Changes only its own window.
"""
import argparse
import json
import math
import os
from pathlib import Path
import re
import selectors
import shlex
import socket
import subprocess
import tempfile
import time

HERE = Path(__file__).resolve().parent
OUT = HERE / "measurements"
BIN = HERE / "target/release/spike-gui"
FRAMES = 600
WIDTH, HEIGHT, SCALE = 1800, 1400, 2


def session_env():
    env = dict(os.environ)
    runtime = Path(env["XDG_RUNTIME_DIR"])
    if not env.get("HYPRLAND_INSTANCE_SIGNATURE"):
        matches = [p for p in (runtime / "hypr").iterdir() if (p / ".socket.sock").exists()]
        if len(matches) != 1:
            raise RuntimeError("Select HYPRLAND_INSTANCE_SIGNATURE explicitly")
        env["HYPRLAND_INSTANCE_SIGNATURE"] = matches[0].name
    if not env.get("WAYLAND_DISPLAY"):
        displays = [p.name for p in runtime.glob("wayland-*") if p.is_socket()]
        if len(displays) != 1:
            raise RuntimeError("Select WAYLAND_DISPLAY explicitly")
        env["WAYLAND_DISPLAY"] = displays[0]
    return env


def hypr(env, *args):
    p = subprocess.run(["hyprctl", *args], env=env, text=True, capture_output=True, timeout=5, check=True)
    if p.stdout.startswith("error:"):
        raise RuntimeError(p.stdout)
    return p.stdout


def read_until(fd, predicate, timeout, log):
    selector = selectors.DefaultSelector()
    selector.register(fd, selectors.EVENT_READ)
    deadline = time.monotonic() + timeout
    content = b""
    try:
        while not predicate(content):
            remaining = deadline - time.monotonic()
            if remaining <= 0 or not selector.select(remaining):
                raise TimeoutError("Timed out waiting for measurement readiness")
            data = os.read(fd, 65536)
            if not data:
                raise RuntimeError("Measurement process exited before readiness")
            content += data
            log.write(data)
            log.flush()
        return content
    finally:
        selector.close()


def launch(env, name):
    log = (OUT / f"{name}.log").open("wb")
    overrides = {k: v for k, v in env.items() if k.startswith(("SPIKE_", "ZED_")) or k == "WAYLAND_DISPLAY"}
    log.write((time.strftime("UTC start: %Y-%m-%dT%H:%M:%SZ\n", time.gmtime()) +
               "Environment: " + json.dumps(overrides, sort_keys=True) + "\n" +
               "Command: " + str(BIN) + "\n").encode())
    events = socket.socket(socket.AF_UNIX)
    events.settimeout(15)
    events.connect(str(Path(env["XDG_RUNTIME_DIR"]) / "hypr" / env["HYPRLAND_INSTANCE_SIGNATURE"] / ".socket2.sock"))
    p = subprocess.Popen([str(BIN)], cwd=HERE, env=env, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
    try:
        deadline = time.monotonic() + 15
        while True:
            clients = json.loads(hypr(env, "-j", "clients"))
            client = next((c for c in clients if c["pid"] == p.pid), None)
            if client:
                break
            events.settimeout(max(0.001, deadline - time.monotonic()))
            events.recv(65536)
        target = f"address:{client['address']}"
        operations = [
            f"hl.dsp.window.float({{window='{target}',action='set'}})",
            f"hl.dsp.window.set_prop({{window='{target}',prop='no_anim',value='true'}})",
            f"hl.dsp.window.resize({{window='{target}',x={WIDTH},y={HEIGHT},relative=false}})",
            f"hl.dsp.window.move({{window='{target}',x=12,y=38,relative=false}})",
        ]
        code = "; ".join(f"hl.dispatch({op})" for op in operations)
        result = hypr(env, "eval", code)
        log.write(("hyprctl eval " + shlex.quote(code) + "\n" + result).encode())
        expected = f"SPIKE_GEOMETRY viewport={WIDTH}x{HEIGHT} scale={SCALE} ".encode()
        read_until(p.stderr.fileno(), lambda b: any(expected in line and b"fits=true" in line for line in b.splitlines()), 15, log)
        client = next(c for c in json.loads(hypr(env, "-j", "clients")) if c["pid"] == p.pid)
        (OUT / f"{name}-window.json").write_text(json.dumps(client, indent=2) + "\n")
        return p, log
    except BaseException:
        p.terminate()
        p.wait(timeout=10)
        log.write(p.stderr.read())
        log.close()
        raise
    finally:
        events.close()


def finish(p, log, timeout=45):
    try:
        _, stderr = p.communicate(timeout=timeout)
        log.write(stderr)
        log.write(f"\nExit status: {p.returncode}\n".encode())
        if p.returncode:
            raise RuntimeError(f"GUI exited {p.returncode}")
    finally:
        if p.poll() is None:
            p.terminate()
            p.wait(timeout=10)
        log.close()


def record_power():
    """Power state changes results a lot on this laptop; keep it with every run."""
    def read(path):
        try:
            return Path(path).read_text().strip()
        except OSError:
            return "unavailable"
    supplies = {p.name: read(p / "online") for p in Path("/sys/class/power_supply").glob("*") if (p / "online").exists()}
    profile = subprocess.run(["powerprofilesctl", "get"], text=True, capture_output=True).stdout.strip() or "unavailable"
    cpu = Path("/sys/devices/system/cpu/cpu0/cpufreq")
    state = {"utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()), "power_supply_online": supplies,
             "power_profile": profile, "governor": read(cpu / "scaling_governor"),
             "energy_performance_preference": read(cpu / "energy_performance_preference")}
    with (OUT / "power.jsonl").open("a") as f:
        f.write(json.dumps(state) + "\n")
    print("Power:", json.dumps(state), flush=True)


def frames(env):
    for renderer, mode in [("glyph", "static"), ("glyph", "styled"), ("glyph", "typing"), ("glyph", "scroll"), ("shape", "styled")]:
        run_env = {**env, "SPIKE_BENCH": str(FRAMES), "SPIKE_BENCH_MODE": mode,
                   "SPIKE_RENDER": renderer, "SPIKE_CMD": "read -r -t 60 line", "ZED_MEASUREMENTS": "1"}
        p, log = launch(run_env, f"frames-{renderer}-{mode}")
        finish(p, log)
        print(f"Completed frames-{renderer}-{mode}", flush=True)


def cat_idle(env):
    # Regenerate exactly 50 MiB, retaining the last partial line in the byte count.
    line = b"2026-10-25T00:00:00Z INFO request=123456 path=/api/build status=200 elapsed_ms=2 " + b"x" * 60 + b"\n"
    data = Path(tempfile.gettempdir()) / "shoal-t00-50mb.log"
    size = 50 * 1024 * 1024
    count, remainder = divmod(size, len(line))
    data.write_bytes(line * count + line[:remainder])
    print(f"Data: {data}, {size} bytes, {count} newline-terminated lines, {remainder} trailing bytes", flush=True)
    for run in range(1, 4):
        with tempfile.TemporaryDirectory(prefix="shoal-t00-") as tmp:
            paths = {key: Path(tmp) / key for key in ("start", "done", "stop")}
            fds = {}
            try:
                for key, path in paths.items():
                    os.mkfifo(path, 0o600)
                    fds[key] = os.open(path, os.O_RDWR | os.O_NONBLOCK)
                cmd = (f"read -r -t 30 < {shlex.quote(str(paths['start']))}; "
                       f"s=$EPOCHREALTIME; cat {shlex.quote(str(data))}; status=$?; e=$EPOCHREALTIME; "
                       f"printf '%s %s %s\\n' \"$s\" \"$e\" \"$status\" > {shlex.quote(str(paths['done']))}; "
                       f"read -r -t 60 < {shlex.quote(str(paths['stop']))}")
                run_env = {**env, "SPIKE_CMD": cmd, "SPIKE_STATS": "1", "ZED_MEASUREMENTS": "1"}
                p, log = launch(run_env, f"cat-{run}")
                try:
                    os.write(fds["start"], b"go\n")
                    with (OUT / f"cat-{run}-time.txt").open("wb") as timing:
                        read_until(fds["done"], lambda b: b"\n" in b, 30, timing)
                    # A final frame drains queued output. Keep process idle for the full pidstat sample.
                    if run == 1:
                        with (OUT / "idle-30s.txt").open("w") as cpu:
                            subprocess.run(["pidstat", "-u", "-p", str(p.pid), "1", "30"], stdout=cpu, stderr=subprocess.STDOUT, check=True, timeout=40)
                        status = Path(f"/proc/{p.pid}/status").read_text()
                        (OUT / "idle-process-status.txt").write_text(status)
                        (OUT / "idle-threads.json").write_text(json.dumps([
                            {"tid": t.name, "stat": (t / "stat").read_text(), "comm": (t / "comm").read_text()}
                            for t in Path(f"/proc/{p.pid}/task").iterdir()
                        ], indent=2) + "\n")
                    else:
                        # The finite sample also lets final VT input/paint drain without sleeps.
                        with (OUT / f"cat-{run}-settle.txt").open("w") as cpu:
                            subprocess.run(["pidstat", "-u", "-p", str(p.pid), "1", "1"], stdout=cpu, stderr=subprocess.STDOUT, check=True, timeout=5)
                    os.write(fds["stop"], b"stop\n")
                    finish(p, log)
                except BaseException:
                    if p.poll() is None:
                        p.terminate()
                        p.wait(timeout=10)
                    if not log.closed:
                        log.write(p.stderr.read())
                        log.close()
                    raise
                print(f"Completed cat-{run}", flush=True)
            finally:
                for fd in fds.values():
                    os.close(fd)

def summarize():
    results = {}
    for path in sorted(OUT.glob("frames-*.log")) + sorted(OUT.glob("cat-[123].log")):
        text = path.read_text()
        # The summary is printed inside the final quit frame. Discard that
        # frame and the 30 warmups; take the last 600 completed GPUI frames.
        body = text.split("BENCH grid")[0]
        matches = re.findall(r"frame duration: ([0-9.]+)(ms|µs|ns|s)", body)
        values = [float(n) * {"ms": 1, "µs": .001, "ns": .000001, "s": 1000}[unit]
                  for n, unit in matches]
        if path.name.startswith("frames-"):
            values = values[-FRAMES:]
        ordered = sorted(values)
        def pct(q):
            return ordered[math.floor((len(ordered) - 1) * q + .5)]
        result = {
            "gpui_n": len(values),
            "gpui_p50_ms": pct(.5), "gpui_p95_ms": pct(.95),
            "gpui_p99_ms": pct(.99), "gpui_max_ms": max(values),
            "internal": [line for line in text.splitlines()
                         if line.startswith(("build draw", "shape +", "render start", "resolve rows",
                                             "paint (canvas)", "rows ", "glyphs painted", "bench output",
                                             "SPIKE_VERIFY", "PTY bytes", "feed:", "  of which"))],
            "geometry": [line for line in text.splitlines() if line.startswith("SPIKE_GEOMETRY")],
        }
        if path.name.startswith("cat-"):
            start, end, status = (OUT / f"{path.stem}-time.txt").read_text().split()
            # Decimal avoids precision loss when subtracting epoch timestamps.
            from decimal import Decimal
            result["cat_seconds"] = str(Decimal(end) - Decimal(start))
            result["cat_exit"] = int(status)
            samples = []
            for at, fed, paint, total in re.findall(
                r"SPIKE_FRAME unix_us=(\d+) feed_bytes=(\d+) paint_us=(\d+) total_us=(\d+)", text
            ):
                if int(fed) > 0:
                    samples.append({"unix_us": int(at), "feed_bytes": int(fed),
                                    "paint_ms": int(paint) / 1000, "total_ms": int(total) / 1000})
            during = [s for s in samples
                      if Decimal(start) <= Decimal(s["unix_us"]) / 1_000_000 <= Decimal(end)]
            result["output_paints"] = samples
            result["paints_ending_while_cat_writes"] = len(during)
            result["while_cat_writes"] = during
        results[path.stem] = result
    (OUT / "summary.json").write_text(json.dumps(results, indent=2) + "\n")
    print(json.dumps(results, indent=2))



if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("measure", choices=["frames", "cat-idle", "summary"])
    parser.add_argument("--out", type=Path, help="evidence folder (default: spike/measurements)")
    parser.add_argument("--bin", type=Path, help="spike-gui binary (default: target/release)")
    parser.add_argument("--frames", type=int, default=FRAMES, help="reported frames per mode")
    parser.add_argument("--verify", action="store_true", help="set SPIKE_VERIFY_ROWS=1 (not for timing)")
    parser.add_argument("--flood-frame-ms", type=int, help="set SPIKE_FLOOD_FRAME_MS")
    args = parser.parse_args()
    if args.out:
        OUT = args.out.resolve()
    if args.bin:
        BIN = args.bin.resolve()
    FRAMES = args.frames
    OUT.mkdir(parents=True, exist_ok=True)
    if args.measure == "summary":
        summarize()
        raise SystemExit(0)
    env = {**session_env(), "SPIKE_GRID": "200x60", "SPIKE_WINDOW": f"{WIDTH}x{HEIGHT}", "SPIKE_FONT_PX": "14"}
    # Inherited benchmark flags would change the workload being measured.
    for key in ("SPIKE_BENCH", "SPIKE_BENCH_MODE", "SPIKE_NO_LAYER", "SPIKE_RENDER", "SPIKE_STATS", "SPIKE_FONT",
                "SPIKE_VERIFY_ROWS", "SPIKE_FLOOD_FRAME_MS", "ZED_MEASUREMENTS"):
        env.pop(key, None)
    if args.verify:
        env["SPIKE_VERIFY_ROWS"] = "1"
    if args.flood_frame_ms is not None:
        env["SPIKE_FLOOD_FRAME_MS"] = str(args.flood_frame_ms)
    record_power()
    {"frames": frames, "cat-idle": cat_idle}[args.measure](env)
    record_power()
