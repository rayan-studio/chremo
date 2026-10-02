"""Runtime regression for Web Animations feature detection and CSS animation.

Run after building: python support/test_web_animation_exposure.py
Uses the standard library and the built servoshell WebDriver server.
"""

import argparse
import json
from pathlib import Path
import socket
import subprocess
import tempfile
import time
import urllib.error
import urllib.parse
import urllib.request


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, default=Path("target/medium/servoshell.exe"))
    parser.add_argument("--pref", choices=("true", "false"))
    args = parser.parse_args()
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        port = sock.getsockname()[1]
    base = f"http://127.0.0.1:{port}"
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))

    def request(method, path, body=None):
        data = None if body is None else json.dumps(body).encode()
        req = urllib.request.Request(base + path, data=data, method=method,
                                     headers={"Content-Type": "application/json"})
        with opener.open(req, timeout=20) as response:
            result = json.load(response)["value"]
        if isinstance(result, dict) and "error" in result:
            raise AssertionError(result)
        return result

    command = [str(args.binary.resolve()), "--headless", f"--webdriver={port}"]
    if args.pref is not None:
        command += [f"--pref=dom_web_animations_enabled={args.pref}"]
    command += ["about:blank"]
    with tempfile.TemporaryFile() as log:
        process = subprocess.Popen(command, stdout=log, stderr=log)
        session = None
        try:
            deadline = time.monotonic() + 30
            while True:
                try:
                    request("GET", "/status")
                    break
                except (OSError, urllib.error.URLError):
                    if process.poll() is not None or time.monotonic() > deadline:
                        raise RuntimeError("WebDriver did not start")
                    time.sleep(0.1)
            session = request("POST", "/session", {"capabilities": {}})["sessionId"]
            prefix = f"/session/{session}"
            html = """<!doctype html><style>
                @keyframes reveal { from { opacity: 0.2 } to { opacity: 0.8 } }
                #box { width: 20px; height: 20px; background: red;
                       animation: reveal 100ms linear forwards; }
                </style><div id="box"></div>"""
            request("POST", prefix + "/url", {
                "url": "data:text/html," + urllib.parse.quote(html)})

            def evaluate(script):
                return request("POST", prefix + "/execute/sync", {"script": script, "args": []})

            result = evaluate("""return {
                animate: typeof Element.prototype.animate,
                animation: typeof Animation,
                keyframeEffect: typeof KeyframeEffect,
                mediaPause: typeof HTMLMediaElement.prototype.pause,
                fallback: !(typeof Element.prototype.animate === 'function')
            };""")
            enabled = args.pref == "true"
            expected = "function" if enabled else "undefined"
            for key in ("animate", "animation", "keyframeEffect"):
                assert result[key] == expected, (key, result)
            assert result["mediaPause"] == "function", result
            assert result["fallback"] == (not enabled), result
            deadline = time.monotonic() + 5
            while True:
                opacity = float(evaluate("return getComputedStyle(document.getElementById('box')).opacity;"))
                if abs(opacity - 0.8) < 0.01:
                    break
                assert time.monotonic() < deadline, ("CSS animation did not finish", opacity)
                time.sleep(0.05)
            print("PASS: API exposure, feature-detection fallback, media pause, CSS animation")
        except Exception:
            log.seek(0)
            print(log.read().decode(errors="replace")[-4000:])
            raise
        finally:
            if session is not None:
                try:
                    request("DELETE", f"/session/{session}")
                except OSError:
                    pass
            if process.poll() is None:
                process.terminate()
                try:
                    process.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait()


if __name__ == "__main__":
    main()
