"""
localguard.py - keep other websites out of the local tools' web servers.

label_tool.py and tracker.py serve pages on 127.0.0.1. Without a check, any website open
in your browser could send them requests (for example, a hidden form that overwrites your
labels), or use "DNS rebinding" to read their answers. Every request must therefore:

  * name this computer in its Host header (127.0.0.1 or localhost, with the right port), which
    defeats DNS rebinding, and
  * for anything that changes or computes something (POST): carry the custom header
    X-Face-Tool: 1 and, if the browser sends an Origin, come from this tool's own page.
    Browsers won't let another website add a custom header without the server's permission
    (a CORS preflight these servers never grant), so only the tool's own pages can do it.
"""

HEADER = "X-Face-Tool"


def refuse_reason(headers, port, write):
    """Return why a request must be refused, or None if it may proceed."""
    local = {f"127.0.0.1:{port}", f"localhost:{port}"}
    if headers.get("Host", "") not in local:
        return "This tool only answers requests addressed to 127.0.0.1."
    if write:
        if headers.get(HEADER) != "1":
            return f"Missing the {HEADER} header; requests must come from the tool's own page."
        origin = headers.get("Origin")
        if origin is not None and origin not in {f"http://{h}" for h in local}:
            return "Requests from other websites are not allowed."
    return None
