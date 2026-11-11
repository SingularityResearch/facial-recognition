"""get_yunet_dynamic.py - make models/face_detection_yunet_2023mar_dynamic.onnx: the YuNet face detector (OpenCV Zoo,
MIT licence) with any input size, so ONNX Runtime can run it on a GPU (face_id._yunet_runner). OpenCV's detector
resizes the fixed 640x640 model itself; ONNX Runtime needs the sizes marked free. Only the input and output shapes
change (to named sizes); every layer and weight is the same, and the model's reshapes already take any size (-1).

Needs the onnx package (pip install onnx; not in requirements.txt). The result is published in the models-v1
release, which fetch_models.py downloads it from, so users don't run this.
Run:  python tools/get_yunet_dynamic.py
"""
import hashlib
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
import face_id as F  # noqa: E402


def main():
    import onnx
    m = onnx.load(str(F.DET_MODEL))
    dims = m.graph.input[0].type.tensor_type.shape.dim
    dims[2].dim_param, dims[3].dim_param = "height", "width"
    for out in m.graph.output:
        out.type.tensor_type.shape.dim[1].dim_param = "anchors"
    del m.graph.value_info[:]                   # inner shapes worked out for 640x640: ONNX Runtime infers them again
    onnx.checker.check_model(m)
    onnx.save(m, str(F.DET_DYNAMIC))
    data = F.DET_DYNAMIC.read_bytes()
    print(f"{F.DET_DYNAMIC.name}: {len(data)} bytes, SHA-256 {hashlib.sha256(data).hexdigest()}")


if __name__ == "__main__":
    main()
