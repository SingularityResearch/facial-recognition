"""
get_adaface.py - download the official AdaFace IR-101 (WebFace12M) weights and convert them to ONNX.

Weights: https://huggingface.co/minchul/cvlface_adaface_ir101_webface12m (Minchul Kim, AdaFace author).
Only the weight file is downloaded; it is loaded with torch.load(weights_only=True), which can't run
code. The network below is written out here (same layer layout as the official IR-101), so no
downloaded code is executed. Needs PyTorch (installed). Writes models/adaface_ir101_webface12m.onnx.
Run:  python tools/get_adaface.py
"""
import sys
import urllib.request
from pathlib import Path

import numpy as np
import torch
from torch import nn

HERE = Path(__file__).resolve().parent.parent
WEIGHTS = HERE / "models" / "adaface_src" / "model.pt"
OUT = HERE / "models" / "adaface_ir101_webface12m.onnx"
URL = "https://huggingface.co/minchul/cvlface_adaface_ir101_webface12m/resolve/main/pretrained_model/model.pt"


class Flatten(nn.Module):
    def forward(self, x):
        return x.reshape(x.size(0), -1)


class BasicBlockIR(nn.Module):
    """Improved-residual block: BN-conv3x3-BN-PReLU-conv3x3-BN, plus shortcut."""

    def __init__(self, cin, depth, stride):
        super().__init__()
        if cin == depth:
            self.shortcut_layer = nn.MaxPool2d(1, stride)
        else:
            self.shortcut_layer = nn.Sequential(nn.Conv2d(cin, depth, 1, stride, bias=False), nn.BatchNorm2d(depth))
        self.res_layer = nn.Sequential(
            nn.BatchNorm2d(cin), nn.Conv2d(cin, depth, 3, 1, 1, bias=False), nn.BatchNorm2d(depth),
            nn.PReLU(depth), nn.Conv2d(depth, depth, 3, stride, 1, bias=False), nn.BatchNorm2d(depth))

    def forward(self, x):
        return self.res_layer(x) + self.shortcut_layer(x)


class IR101(nn.Module):
    """IResNet-100 backbone for 112x112 faces -> 512-number embedding (AdaFace 'IR_101')."""

    STAGES = [(64, 64, 3), (64, 128, 13), (128, 256, 30), (256, 512, 3)]   # (in, out, units)

    def __init__(self):
        super().__init__()
        self.input_layer = nn.Sequential(nn.Conv2d(3, 64, 3, 1, 1, bias=False), nn.BatchNorm2d(64), nn.PReLU(64))
        blocks = []
        for cin, depth, units in self.STAGES:
            blocks.append(BasicBlockIR(cin, depth, 2))
            blocks += [BasicBlockIR(depth, depth, 1) for _ in range(units - 1)]
        self.body = nn.Sequential(*blocks)
        self.output_layer = nn.Sequential(nn.BatchNorm2d(512), nn.Dropout(0.4), Flatten(),
                                          nn.Linear(512 * 7 * 7, 512), nn.BatchNorm1d(512, affine=False))

    def forward(self, x):
        return self.output_layer(self.body(self.input_layer(x)))


class Affine(nn.Module):
    """Per-channel x * scale + shift: what a BatchNorm layer computes at inference time."""

    def __init__(self, scale, shift, dims):
        super().__init__()
        shape = (1, -1) + (1,) * (dims - 2)
        self.register_buffer("scale", scale.reshape(shape).clone())
        self.register_buffer("shift", shift.reshape(shape).clone())

    def forward(self, x):
        return x * self.scale + self.shift


def batchnorm_to_affine(module):
    """Replace every BatchNorm with the equivalent multiply-add. Same maths, and it avoids a
    DirectML (GPU) bug with BatchNormalization nodes that don't follow a convolution."""
    for name, child in module.named_children():
        if isinstance(child, (nn.BatchNorm1d, nn.BatchNorm2d)):
            s = 1.0 / torch.sqrt(child.running_var + child.eps)
            w = child.weight.detach() if child.affine else torch.ones_like(s)
            b = child.bias.detach() if child.affine else torch.zeros_like(s)
            scale = w * s
            setattr(module, name, Affine(scale, b - child.running_mean * scale, 2 if isinstance(child, nn.BatchNorm1d) else 4))
        else:
            batchnorm_to_affine(child)


def main():
    if not WEIGHTS.exists():
        WEIGHTS.parent.mkdir(parents=True, exist_ok=True)
        print("downloading AdaFace IR-101 weights (261 MB)...", flush=True)
        urllib.request.urlretrieve(URL, WEIGHTS)
    state = torch.load(WEIGHTS, map_location="cpu", weights_only=True)
    state = {k[4:] if k.startswith("net.") else k: v for k, v in state.items()}
    net = IR101()
    net.load_state_dict(state, strict=True)       # fails loudly if any layer doesn't match
    net.eval()
    with torch.no_grad():
        check = net(torch.randn(2, 3, 112, 112, generator=torch.Generator().manual_seed(0)))
    batchnorm_to_affine(net)
    with torch.no_grad():
        after = net(torch.randn(2, 3, 112, 112, generator=torch.Generator().manual_seed(0)))
    print(f"BatchNorm -> multiply-add conversion: max difference {float((check - after).abs().max()):.2e}")

    x = torch.randn(4, 3, 112, 112)
    with torch.no_grad():
        ref = net(x).numpy()
    kwargs = dict(input_names=["input"], output_names=["embedding"], opset_version=17,
                  dynamic_axes={"input": {0: "batch"}, "embedding": {0: "batch"}})
    try:
        torch.onnx.export(net, x, str(OUT), dynamo=False, **kwargs)
    except TypeError:
        torch.onnx.export(net, x, str(OUT), **kwargs)

    import onnxruntime as ort
    out = ort.InferenceSession(str(OUT), providers=["CPUExecutionProvider"]).run(None, {"input": x.numpy()})[0]
    cos = np.sum(out * ref, 1) / np.linalg.norm(out, axis=1) / np.linalg.norm(ref, axis=1)
    print(f"saved {OUT.name} ({OUT.stat().st_size / 1e6:.0f} MB); ONNX vs PyTorch agreement {cos.min():.6f}")


if __name__ == "__main__":
    sys.exit(main())
