"""Every self.<name> a class reads is set somewhere in it (or is a method): drawing a face in the photo view failed
from v1.10 to v1.11 because Importer._more_lock was removed while _points_in still used it, on a path the other tests
don't reach without the face models."""
import ast
from pathlib import Path

import pytest

HERE = Path(__file__).resolve().parent.parent


def _unset_reads(path):
    tree = ast.parse(path.read_text("utf-8"))
    out = []
    for cls in (n for n in tree.body if isinstance(n, ast.ClassDef)):
        if any(not (isinstance(b, ast.Name) and b.id == "object") for b in cls.bases):
            continue                              # a subclass: its base sets the rest
        known = {n.name for n in cls.body if isinstance(n, (ast.FunctionDef, ast.AsyncFunctionDef))}
        known |= {t.id for n in cls.body if isinstance(n, (ast.Assign, ast.AnnAssign))
                  for t in (n.targets if isinstance(n, ast.Assign) else [n.target]) if isinstance(t, ast.Name)}
        reads = []
        for node in ast.walk(cls):
            if isinstance(node, ast.Attribute) and isinstance(node.value, ast.Name) and node.value.id == "self":
                if isinstance(node.ctx, ast.Store):
                    known.add(node.attr)
                else:
                    reads.append(node.attr)
        out += [f"{cls.name}.{a}" for a in sorted(set(reads) - known)]
    return out


@pytest.mark.parametrize("module", sorted(p.name for p in HERE.glob("*.py")))
def test_every_attribute_read_is_set(module):
    assert _unset_reads(HERE / module) == []
