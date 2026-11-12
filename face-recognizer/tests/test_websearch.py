"""websearch.py: key checks, where keys are kept, and the monthly spending caps (no network)."""
import json
import threading

import pytest

import websearch as W

AWS_ID, AWS_SECRET = "AKIA" + "A" * 16, "a" * 40


def test_google_key(home):
    with pytest.raises(ValueError):
        W.save_key("not a key")
    W.save_key("A" * 39)
    assert W.key() == "A" * 39 and (home / "google_vision_key.txt").exists()
    W.save_key("")                                          # empty removes it
    assert W.key() is None and not (home / "google_vision_key.txt").exists()


def test_keys_kept_protected_and_old_plain_ones_converted(home):
    W.save_key("B" * 39)
    W.save_aws(AWS_ID, AWS_SECRET, "us-east-1")
    assert "B" * 39 not in (home / "google_vision_key.txt").read_text("utf-8")
    assert AWS_SECRET not in (home / "aws_rekognition.json").read_text("utf-8")
    assert W.key() == "B" * 39 and W.aws()["secret_access_key"] == AWS_SECRET
    (home / "google_vision_key.txt").write_text("C" * 39, "utf-8")             # saved plain by an older version
    (home / "aws_rekognition.json").write_text(json.dumps({"access_key_id": AWS_ID, "secret_access_key": "b" * 40,
                                                           "region": "us-east-1"}), "utf-8")
    assert W.key() == "C" * 39 and W.aws()["secret_access_key"] == "b" * 40   # still read...
    assert "C" * 39 not in (home / "google_vision_key.txt").read_text("utf-8")  # ...and now kept protected
    assert "b" * 40 not in (home / "aws_rekognition.json").read_text("utf-8")


@pytest.mark.parametrize("key_id, secret, region", [
    ("BADKEY", AWS_SECRET, "us-east-1"), (AWS_ID, "short", "us-east-1"), (AWS_ID, AWS_SECRET, "mars"),
])
def test_amazon_key_refused(home, key_id, secret, region):
    with pytest.raises(ValueError):
        W.save_aws(key_id, secret, region)
    assert W.aws() is None


def test_amazon_key_saved_and_removed(home):
    W.save_aws(AWS_ID, AWS_SECRET, "us-west-2")
    assert W.aws() == {"access_key_id": AWS_ID, "secret_access_key": AWS_SECRET, "region": "us-west-2"}
    assert W.services() == {"vision": False, "rekognition": True}
    W.save_aws("", "", "")
    assert W.aws() is None


def test_cap_holds_with_searches_at_once(home, monkeypatch):
    monkeypatch.setitem(W.CAPS, "vision", 5)
    ok, refused = [], []

    def search():
        try:
            W._count("vision")
            ok.append(1)
        except ValueError:
            refused.append(1)
    threads = [threading.Thread(target=search) for _ in range(20)]
    for t in threads:
        t.start()
    for t in threads:
        t.join()
    assert len(ok) == 5 and len(refused) == 15 and W.usage("vision")["used"] == 5


def test_count_starts_again_each_month(home):
    (home / "vision_usage.json").write_text(json.dumps({"month": "1999-01", "used": 999}), "utf-8")
    assert W.usage("vision")["used"] == 0
