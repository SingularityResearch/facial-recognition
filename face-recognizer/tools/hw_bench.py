"""
hw_bench.py - measure the face models on this computer's CPU, GPU and NPU.

Run with the Ryzen AI environment's Python (it has ONNX Runtime with the NPU and GPU providers):
  X:\\ProgramData\\miniconda3\\envs\\ryzen-ai-1.8.0\\python.exe tools\\hw_bench.py

For each processor it reports milliseconds per face and how closely the face prints match
the CPU's (cosine similarity 1.000 = identical), since the NPU computes in 16-bit BF16.
"""
import json
import os
import sys
import time
from pathlib import Path

import numpy as np
import onnx
import onnxruntime as ort

HERE = Path(__file__).resolve().parent.parent
MODELS = HERE / "models"
NPU_DIR = MODELS / "npu"
NPU_DIR.mkdir(exist_ok=True)


def static_copy(src, batch=1):
    """The NPU compiler needs fixed tensor shapes: pin the batch dimension."""
    dst = NPU_DIR / f"{src.stem}_b{batch}.onnx"
    if not dst.exists():
        m = onnx.load(str(src))
        for t in list(m.graph.input) + list(m.graph.output):
            d = t.type.tensor_type.shape.dim
            if d and not d[0].dim_value:
                d[0].dim_value = batch
        m = onnx.shape_inference.infer_shapes(m)
        onnx.save(m, str(dst))
    return dst


def faces(n):
    """n aligned 112x112 face crops from the face database, as BGR uint8."""
    import cv2
    z = np.load(HERE / "data" / "faces.npz", allow_pickle=False)
    jpg, off = z["thumbs"], z["thumb_off"]
    idx = np.linspace(0, len(off) - 2, n).astype(int)
    return [cv2.imdecode(jpg[off[i]:off[i + 1]], cv2.IMREAD_COLOR) for i in idx]


def arcface_input(crops):
    x = np.stack([c[:, :, ::-1] for c in crops]).astype(np.float32)      # BGR -> RGB
    return ((x - 127.5) / 127.5).transpose(0, 3, 1, 2).copy()


def session(model, provider):
    so = ort.SessionOptions()
    so.log_severity_level = 3
    if provider == "NPU":
        cfg = NPU_DIR / "vitisai_config.json"
        cfg.write_text(json.dumps({
            "passes": [{"name": "init", "plugin": "vaip-pass_init"},
                       {"name": "vaiml_partition", "plugin": "vaip-pass_vaiml_partition",
                        "vaiml_config": {"enable_f32_to_bf16_conversion": True, "preferred_data_storage": "auto"}}],
            "target": "VAIML",
            "targets": [{"name": "VAIML", "pass": ["init", "vaiml_partition"]}]}))
        opts = {"config_file": str(cfg), "cache_dir": str(NPU_DIR / "cache"), "cache_key": model.stem,
                "enable_cache_file_io_in_mem": 0}
        return ort.InferenceSession(str(model), so, providers=["VitisAIExecutionProvider"], provider_options=[opts])
    if provider == "GPU":
        return ort.InferenceSession(str(model), so, providers=["DmlExecutionProvider"])
    return ort.InferenceSession(str(model), so, providers=["CPUExecutionProvider"])


def bench(sess, x, reps=30):
    name = sess.get_inputs()[0].name
    out = [sess.run(None, {name: x[i:i + 1]})[0][0] for i in range(len(x))]       # one face at a time
    t = time.perf_counter()
    for r in range(reps):
        sess.run(None, {name: x[r % len(x):r % len(x) + 1]})
    ms = (time.perf_counter() - t) / reps * 1000
    out = np.asarray(out, np.float32)
    return ms, out / np.linalg.norm(out, axis=1, keepdims=True)


def main():
    src = MODELS / "arcface_w600k_r50.onnx"
    if not src.exists():
        sys.exit("arcface_w600k_r50.onnx not found; start the tracker once so it rebuilds it from its parts.")
    model = static_copy(src)
    x = arcface_input(faces(20))
    print("ONNX Runtime", ort.__version__, "providers:", ort.get_available_providers())
    results, ref = {}, None
    for proc in ("CPU", "GPU", "NPU"):
        try:
            t0 = time.perf_counter()
            s = session(model, proc)
            setup = time.perf_counter() - t0
            used = s.get_providers()[0]
            ms, emb = bench(s, x)
            if ref is None:
                ref = emb
            agree = float(np.mean(np.sum(emb * ref, axis=1)))
            results[proc] = dict(ms=round(ms, 2), agreement=round(agree, 4), setup_s=round(setup, 1), provider=used)
            print(f"{proc}: {ms:6.2f} ms/face  agreement with CPU {agree:.4f}  setup {setup:5.1f}s  ({used})", flush=True)
        except Exception as e:
            results[proc] = dict(error=str(e)[:300])
            print(f"{proc}: failed: {str(e)[:300]}", flush=True)
    (HERE / "data" / "hw_bench.json").write_text(json.dumps(results, indent=1))


if __name__ == "__main__":
    main()
