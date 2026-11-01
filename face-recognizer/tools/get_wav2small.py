"""
get_wav2small.py - download audEERING's Wav2Small voice model and convert it to ONNX.

Wav2Small (https://huggingface.co/audeering/wav2small, paper arXiv:2408.13920) reads arousal,
dominance and valence (each about 0..1) from 16 kHz speech. It has 17K parameters, so it runs in
about a millisecond a second of audio on the CPU. License: CC BY-NC-SA 4.0, research / non-commercial
use only (a commercial license is available from audEERING) - like the ArcFace weights.

Only the weight file (model.safetensors) is downloaded. It's read here as plain numbers (safetensors
is a JSON header + raw arrays, so nothing in it can run), and the network is written out below from
the model card, so no downloaded code is executed. Needs PyTorch (installed). Writes models/wav2small.onnx.
Run:  python tools/get_wav2small.py
"""
import json
import struct
import sys
import urllib.request
from pathlib import Path

import numpy as np
import torch
from torch import nn

HERE = Path(__file__).resolve().parent.parent
WEIGHTS = HERE / "models" / "wav2small_src" / "model.safetensors"
OUT = HERE / "models" / "wav2small.onnx"
URL = "https://huggingface.co/audeering/wav2small/resolve/main/model.safetensors"


def read_safetensors(path):
    """{name: tensor} from a .safetensors file: 8-byte header length, JSON header, raw data."""
    raw = path.read_bytes()
    n = struct.unpack("<Q", raw[:8])[0]
    header = json.loads(raw[8:8 + n])
    kinds = {"F32": np.float32, "I64": np.int64}
    out = {}
    for name, info in header.items():
        if name == "__metadata__":
            continue
        a, b = info["data_offsets"]
        arr = np.frombuffer(raw[8 + n + a:8 + n + b], kinds[info["dtype"]]).reshape(info["shape"])
        out[name] = torch.from_numpy(arr.copy())
    return out


def prenorm(x):
    """Each clip scaled to zero mean, unit variance (as wav2vec2)."""
    x = x - x.mean(1, keepdim=True)
    return x / torch.sqrt((x * x).mean(1, keepdim=True) + 1e-7)


class Spectrogram(nn.Module):
    """Power spectrogram as two fixed convolutions (real and imaginary DFT rows; weights from the file)."""

    def __init__(self, n_fft=64, hop=32):
        super().__init__()
        self.conv_real = nn.Conv1d(1, n_fft // 2 + 1, n_fft, stride=hop, bias=False)
        self.conv_imag = nn.Conv1d(1, n_fft // 2 + 1, n_fft, stride=hop, bias=False)

    def forward(self, x):
        x = x[:, None, :]
        return self.conv_real(x) ** 2 + self.conv_imag(x) ** 2       # batch, freq, frames


class LogMel(nn.Module):
    def __init__(self):
        super().__init__()
        self.register_buffer("melW", torch.zeros(33, 26))
        self.register_buffer("amin", torch.tensor([1e-10]))

    def forward(self, x):
        x = torch.matmul(x[:, None].transpose(2, 3), self.melW)       # batch, 1, frames, mel
        return 10 * torch.log10(torch.maximum(x, self.amin))


class Conv(nn.Module):
    def __init__(self, c_in, c_out):
        super().__init__()
        self.conv = nn.Conv2d(c_in, c_out, 3, padding=1, bias=False)
        self.norm = nn.BatchNorm2d(c_out)

    def forward(self, x):
        return torch.relu(self.norm(self.conv(x)))


class Vgg7(nn.Module):
    def __init__(self):
        super().__init__()
        self.l1, self.l2, self.l3 = Conv(1, 13), Conv(13, 13), Conv(13, 13)
        self.maxpool_A = nn.MaxPool2d(3, stride=2, padding=1)
        self.l4, self.l5, self.l6, self.l7 = Conv(13, 13), Conv(13, 13), Conv(13, 13), Conv(13, 13)
        self.lin = nn.Conv2d(13, 13, 1)
        self.sof = nn.Conv2d(13, 13, 1)
        self.spectrogram_extractor = Spectrogram()
        self.logmel_extractor = LogMel()

    def forward(self, x):
        x = self.logmel_extractor(self.spectrogram_extractor(prenorm(x)))
        x = self.maxpool_A(self.l3(self.l2(self.l1(x))))
        x = self.l7(self.l6(self.l5(self.l4(x))))
        x = (self.lin(x) * self.sof(x).softmax(2)).sum(2)           # attention pooling over time
        x = torch.cat([x, torch.bmm(x, x.transpose(1, 2))], 2)
        return x.reshape(-1, 338)


class Wav2Small(nn.Module):
    def __init__(self):
        super().__init__()
        self.vgg7 = Vgg7()
        self.adv = nn.Linear(338, 3)                                 # 0 arousal, 1 dominance, 2 valence

    def forward(self, x):
        return self.adv(self.vgg7(x))


def main():
    if not WEIGHTS.exists():
        WEIGHTS.parent.mkdir(parents=True, exist_ok=True)
        print(f"Downloading {URL}")
        urllib.request.urlretrieve(URL, WEIGHTS)
    state = {k: v for k, v in read_safetensors(WEIGHTS).items() if not k.endswith("num_batches_tracked")}
    model = Wav2Small().eval()
    missing, unexpected = model.load_state_dict(state, strict=False)
    if missing or unexpected:
        sys.exit(f"Weights don't match the network: missing {missing}, unexpected {unexpected}")

    # A 2-second test signal: a voiced tone with harmonics plus a little noise.
    t = np.arange(32000) / 16000
    sig = (np.sin(2 * np.pi * 140 * t) + 0.5 * np.sin(2 * np.pi * 280 * t)) * (0.6 + 0.4 * np.sin(2 * np.pi * 3 * t))
    sig = torch.from_numpy((sig + 0.02 * np.random.default_rng(0).standard_normal(t.size)).astype(np.float32))[None]
    with torch.no_grad():
        ref = model(sig).numpy()
    torch.onnx.export(model, sig, str(OUT), input_names=["audio"], output_names=["avd"],
                      dynamic_axes={"audio": {1: "samples"}}, opset_version=17, dynamo=False)

    import onnxruntime as ort
    sess = ort.InferenceSession(str(OUT), providers=["CPUExecutionProvider"])
    got = sess.run(None, {"audio": sig.numpy()})[0]
    half = sess.run(None, {"audio": sig.numpy()[:, :16000]})[0]
    print(f"PyTorch arousal/dominance/valence {np.round(ref[0], 3)}, ONNX {np.round(got[0], 3)}; "
          f"1-second clip {np.round(half[0], 3)}")
    if not np.allclose(ref, got, atol=1e-4):
        sys.exit("ONNX output differs from PyTorch")
    print(f"Wrote {OUT} ({OUT.stat().st_size // 1024} KB)")


if __name__ == "__main__":
    main()
