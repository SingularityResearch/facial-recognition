"""64-bit ARM Linux computers (a Raspberry Pi): what the app does differently there. No models and no ARM needed.

Seen on a Raspberry Pi 4 (2026-11-20): MediaPipe's build for ARM ended the whole process the moment a model was
made ("compiled with aes enabled, but this feature is not available on this processor"), so the tracker never
started; and Settings called the processor "unknown processor"."""
import sys

import face_id as F
import hardware
import music

PI4 = """processor\t: 0
BogoMIPS\t: 108.00
Features\t: fp asimd evtstrm crc32 cpuid
CPU implementer\t: 0x41
CPU architecture: 8
CPU variant\t: 0x0
CPU part\t: 0xd08
CPU revision\t: 3

processor\t: 1
Features\t: fp asimd evtstrm crc32 cpuid
CPU implementer\t: 0x41
CPU part\t: 0xd08

Revision\t: d03114
Model\t\t: Raspberry Pi 4 Model B Rev 1.4
"""
PI5 = PI4.replace("fp asimd evtstrm crc32 cpuid",
                  "fp asimd evtstrm aes pmull sha1 sha2 crc32 atomics fphp asimdhp cpuid asimdrdm lrcpc dcpop asimddp"
                  ).replace("0xd08", "0xd0b").replace("Raspberry Pi 4 Model B Rev 1.4", "Raspberry Pi 5 Model B Rev 1.0")


def test_the_processors_features_are_read_from_its_own_list(tmp_path):
    (tmp_path / "pi4").write_text(PI4)
    (tmp_path / "pi5").write_text(PI5)
    (tmp_path / "x86").write_text("processor\t: 0\nmodel name\t: Some x86 processor\nflags\t\t: fpu aes avx2\n")
    assert F.arm_features(tmp_path / "pi4") == {"fp", "asimd", "evtstrm", "crc32", "cpuid"}
    assert {"aes", "pmull", "sha2"} <= F.arm_features(tmp_path / "pi5")
    assert F.arm_features(tmp_path / "x86") is None              # no "Features" line: not an ARM list
    assert F.arm_features(tmp_path / "nothing there") is None


def test_mediapipe_counts_as_not_there_on_an_arm_processor_without_aes(tmp_path):
    """Its build for 64-bit ARM ends the process on such a processor: nothing to catch, so it must not be loaded."""
    pi4, pi5 = {"fp", "asimd", "evtstrm", "crc32", "cpuid"}, {"fp", "asimd", "aes", "pmull", "sha1", "sha2", "crc32"}
    ok, why = F.mediapipe_check(True, "linux", "aarch64", pi4)
    assert not ok and "AES" in why and "Raspberry Pi 4" in why
    assert F.mediapipe_check(True, "linux", "aarch64", pi5) == (True, "")
    assert F.mediapipe_check(True, "linux", "arm64", pi4)[0] is False
    assert F.mediapipe_check(True, "linux", "aarch64", None) == (True, "")      # the list can't be read: as before
    for system, machine in (("linux", "x86_64"), ("win32", "AMD64"), ("win32", "ARM64"), ("darwin", "arm64")):
        assert F.mediapipe_check(True, system, machine, pi4) == (True, "")       # only ARM Linux has that build
    assert F.mediapipe_check(False, "linux", "aarch64", pi5)[0] is False        # not installed at all
    assert "aes" in F.MEDIAPIPE_NEEDS


def test_an_unusable_mediapipe_is_said_once_and_kept_for_settings(monkeypatch, capsys):
    import platform
    monkeypatch.setattr(F, "_MEDIAPIPE", [])
    monkeypatch.setattr(F, "GPU_DECISIONS", {})
    monkeypatch.setattr(F, "_has_module", lambda name: True)
    monkeypatch.setattr(F, "arm_features", lambda *a: {"fp", "asimd", "crc32"})
    monkeypatch.setattr(sys, "platform", "linux")
    monkeypatch.setattr(platform, "machine", lambda: "aarch64")
    assert F.mediapipe_usable()[0] is False and F.mediapipe_usable()[0] is False      # asked once
    out = capsys.readouterr().out
    assert out.count("MediaPipe is not used") == 1 and "AES" in out
    (what, why), = F.GPU_DECISIONS.items()                       # Settings → This computer shows these lines
    assert what.startswith("MediaPipe") and why.startswith("not used: ")
    assert F.mesh_triangles() is None                            # nothing of MediaPipe is asked for
    # a computer that can use it says nothing
    monkeypatch.setattr(F, "_MEDIAPIPE", [])
    monkeypatch.setattr(F, "GPU_DECISIONS", {})
    monkeypatch.setattr(F, "arm_features", lambda *a: {"fp", "asimd", "aes", "crc32"})
    assert F.mediapipe_usable() == (True, "") and F.GPU_DECISIONS == {} and capsys.readouterr().out == ""


def test_the_sound_model_is_not_made_where_mediapipe_cant_run(monkeypatch, tmp_path):
    """music.sounds makes MediaPipe's sound classifier: on such a processor that would end the labeling tool."""
    import numpy as np
    model = tmp_path / "yamnet.tflite"
    model.write_bytes(b"x")
    monkeypatch.setattr(music, "sound_model", lambda: model)
    monkeypatch.setattr(music, "_classifier", {"lock": None, "made": None})
    monkeypatch.setattr(F, "_MEDIAPIPE", [(False, "this processor can't")])
    assert music.sounds(np.zeros(16000 * 2, np.float32), 16000) is None
    assert music._classifier == {"lock": None, "made": None}     # nothing was made, nothing even begun


def test_an_arm_processor_gets_a_name():
    """ARM's /proc/cpuinfo has no "model name": Settings → This computer said "unknown processor" on a Pi."""
    assert hardware.arm_cpu_name(PI4, "Raspberry Pi 4 Model B Rev 1.4\0") == "Raspberry Pi 4 Model B Rev 1.4 (Cortex-A72)"
    assert hardware.arm_cpu_name(PI5, "Raspberry Pi 5 Model B Rev 1.0\0") == "Raspberry Pi 5 Model B Rev 1.0 (Cortex-A76)"
    assert hardware.arm_cpu_name(PI4) == "Cortex-A72"                               # a board that doesn't say its name
    assert hardware.arm_cpu_name(PI4.replace("0xd08", "0xfff")) == "ARM core 0xfff"
    assert hardware.arm_cpu_name(PI4.replace("0x41", "0x51"), "Some board") == "Some board (ARM core 0xd08 (maker 0x51))"
    assert hardware.arm_cpu_name("processor\t: 0\nmodel name\t: x86\n") is None
    assert hardware.arm_cpu_name("", "A board\0") == "A board"


def test_setup_takes_a_newer_system_python_when_there_is_no_3_12():
    """Raspberry Pi OS 13 has Python 3.13 and no python3.12: setup.sh stopped with "Python 3.12 is needed first"."""
    from pathlib import Path
    text = (Path(__file__).resolve().parent.parent / "setup.sh").read_text(encoding="utf-8")
    assert 'PY="${PYTHON:-python3.12}"' in text                  # 3.12 first, and whatever the user names
    assert "sys.version_info >= (3, 12)" in text and "PY=python3\n" in text
    assert '[ "$(uname -m)" = aarch64 ]' in text                  # the Pi's own way: every other system as before
    assert text.index("PY=python3\n") < text.index("is needed first")
