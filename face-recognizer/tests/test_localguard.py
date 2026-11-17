"""localguard: the request checks every server uses, and its bind / error-answer helpers."""
import sys

import pytest

import localguard as G

PORT = 8765


def why(headers, write=False):
    return G.refuse_reason(headers, PORT, write)


@pytest.mark.parametrize("host", ["127.0.0.1:8765", "localhost:8765"])
def test_own_host_is_allowed(host):
    assert why({"Host": host}) is None


@pytest.mark.parametrize("host", ["", "evil.example:8765", "127.0.0.1:8766", "127.0.0.1", "127.0.0.1.evil.example:8765"])
def test_other_hosts_are_refused(host):
    # DNS rebinding: a website's own name pointed at 127.0.0.1 still says its name in Host
    assert why({"Host": host}) is not None


def test_writes_need_the_header_and_own_origin():
    ok = {"Host": "127.0.0.1:8765", "X-Face-Tool": "1"}
    assert why(ok, write=True) is None
    assert why({**ok, "Origin": "http://127.0.0.1:8765"}, write=True) is None
    assert why({"Host": "127.0.0.1:8765"}, write=True) is not None                     # no header
    assert why({**ok, "X-Face-Tool": "yes"}, write=True) is not None
    for origin in ("null", "https://evil.example", "http://127.0.0.1:8764"):
        assert why({**ok, "Origin": origin}, write=True) is not None


@pytest.mark.parametrize("site", ["cross-site"])
def test_requests_from_other_websites_are_refused(site):
    for write in (False, True):
        assert why({"Host": "127.0.0.1:8765", "X-Face-Tool": "1", "Sec-Fetch-Site": site}, write) is not None


@pytest.mark.parametrize("site", ["same-origin", "same-site", "none", None])
def test_own_page_tab_page_and_typed_addresses_are_allowed(site):
    h = {"Host": "127.0.0.1:8765", "X-Face-Tool": "1", **({"Sec-Fetch-Site": site} if site else {})}
    assert why(h) is None and why(h, write=True) is None


@pytest.mark.parametrize("site, header, ok", [("same-origin", None, True), (None, None, True), ("same-site", None, False),
                                              ("none", None, False), ("same-site", "1", True), ("none", "1", True),
                                              ("cross-site", "1", False), ("same-site", "yes", False)])
def test_data_goes_only_to_the_tools_own_page(site, header, ok):
    """A GET for data, or one that starts something (own=True): from the tool's own page (same-origin), or a
    program that sends the header. Not from a page of another program on this computer (same-site) or an address
    typed or opened (none): through 1.24.2 those could start a search of the network or a camera relay."""
    h = {"Host": "127.0.0.1:8765", **({"Sec-Fetch-Site": site} if site else {}), **({"X-Face-Tool": header} if header else {})}
    assert (G.refuse_reason(h, PORT, False, own=True) is None) is ok
    assert (G.refuse_reason(h, PORT, False) is None) is (site != "cross-site")     # the page itself: as before


def test_no_page_trusts_another_to_frame_it_unless_the_app_started_it(monkeypatch):
    assert G.FRAME_PARENTS == set()                           # nothing trusted until app.py adds its tab page
    assert "frame-ancestors 'self'" in G.policy() and "8764" not in G.policy()
    monkeypatch.setattr(G, "FRAME_PARENTS", {"http://127.0.0.1:8764"})
    assert G.policy().endswith("frame-ancestors 'self' http://127.0.0.1:8764")
    assert G.policy("'none'").endswith("frame-ancestors 'none'")


def test_policy_extras_replace_the_shared_rule():
    rules = dict(r.split(" ", 1) for r in G.policy(extra={"script-src": "'self' blob:"}).split("; "))
    assert rules["script-src"] == "'self' blob:" and rules["default-src"] == "'self'" and rules["object-src"] == "'none'"


@pytest.mark.parametrize("length, empty, code", [("-1", False, 400), ("+5", False, 400), ("1e3", False, 400),
                                                 ("", False, 400), ("0", False, 400), ("100", False, 413),
                                                 ("99999", False, 413)])
def test_body_size_refuses_bad_lengths(length, empty, code):
    with pytest.raises(G.RequestError) as e:
        G.body_size({"Content-Length": length}, 100, empty)
    assert e.value.code == code and isinstance(e.value, ValueError)


def test_body_size_takes_good_lengths():
    assert G.body_size({"Content-Length": "99"}, 100) == 99
    assert G.body_size({}, 100, empty=True) == 0 and G.body_size({"Content-Length": " 7 "}, 100) == 7


@pytest.mark.skipif(sys.platform != "win32", reason="address reuse only lets two programs share a port on Windows")
def test_a_second_bind_of_the_same_port_fails():
    first = G.bind(0)
    try:
        with pytest.raises(OSError):
            G.bind(first.server_address[1])
    finally:
        first.server_close()


def test_answer_errors_turns_a_crash_into_a_500():
    sent = []
    G.answer_errors(None, lambda: {}["missing"], lambda body, code: sent.append((code, body)))
    assert sent and sent[0][0] == 500 and "KeyError" in sent[0][1]["error"]
    sent.clear()
    G.answer_errors(None, lambda: None, lambda body, code: sent.append(code))
    assert sent == []
