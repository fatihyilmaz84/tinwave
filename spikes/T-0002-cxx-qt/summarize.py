"""Aggregate results/<prefix>-*.txt: median [min..max] per phase metric. Run: python -I summarize.py [run-deployed|run-noforward]"""
import glob, re, sys, statistics as st
prefix = sys.argv[1] if len(sys.argv) > 1 else "run-deployed"
all_runs = [open(f, encoding="utf8", errors="replace").read() for f in sorted(glob.glob(f"results/{prefix}-*.txt"))]
runs = [r for r in all_runs if "T2 final" in r]
print(f"{len(all_runs)} files, {len(runs)} completed, {len(all_runs)-len(runs)} crashed/incomplete")
phases = {}
for r in runs:
    for m in re.finditer(r"T2 phase (\S+): (.*)", r):
        kv = dict(re.findall(r"(\S+?)=([0-9.]+)", m.group(2)))
        if kv.get("frames") == "0" and not re.match(r"(idle|settle|startup|counter)", m.group(1)):
            continue  # window was occluded / not presenting (no frames): not a valid frame-time sample
        for k, v in kv.items():
            phases.setdefault(m.group(1), {}).setdefault(k, []).append(float(v))
def f(xs): return f"{st.median(xs):.1f} [{min(xs):.1f}..{max(xs):.1f}]"
print(f"runs={len(runs)}")
for name, d in phases.items():
    print(name)
    for k in ["cpu_pct", "frames", "dt_p99", "dt_max", "n>20ms", "n>50ms", "ws_end_mb", "peak_ws_mb", "peak_pws_mb"]:
        if k in d: print(f"   {k:12s} {f(d[k])}")
for key in ["since_main_ms", "process_age_ms", "ws_mb", "pws_mb"]:
    xs = [float(m.group(1)) for r in runs for m in re.finditer(r"first_frame .*?%s=([0-9.]+)" % key, r)]
    print("first_frame", key, f(xs))
for key in ["engine_load_returned since_main_ms"]:
    xs = [float(m.group(1)) for r in runs for m in re.finditer(r"engine_load_returned since_main_ms=([0-9.]+)", r)]
    print(key, f(xs))
xs = [float(m.group(1)) for r in runs for m in re.finditer(r"task_post_latency_us=(\d+)", r)]
print("task_post_latency_us", f(xs), "n=", len(xs))
for key in ["hover", "viewport"]:
    xs = [float(m.group(1)) for r in runs for m in re.finditer(r"%s=(\d+) \| qml indexAt" % key if key=="viewport" else r"qml_to_rust_call_ns hover=(\d+)", r)]
    print("call_ns", key, f(xs))
xs = [float(m.group(1)) for r in runs for m in re.finditer(r"indexAt only=(\d+)", r)]
print("call_ns indexAt only", f(xs))
