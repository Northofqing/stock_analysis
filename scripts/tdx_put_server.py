#!/usr/bin/env python3
"""临时 PUT 接收端：把 Windows VM 里打包好的 tdx_sh.zip / tdx_sz.zip 收到 Mac。

用法：python3 scripts/tdx_put_server.py <port> <dest_dir>
收到文件后打印 `PUT <name> <bytes>`；没有鉴权，只监听局域网 10.211.55.x，
进程由 sync_tdx_lday.sh 启动并在退出时杀掉。
"""
import os
import sys
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer


def main() -> int:
    port = int(sys.argv[1]) if len(sys.argv) > 1 else 8911
    dest = sys.argv[2] if len(sys.argv) > 2 else os.path.expanduser("~/tdx_lday/incoming")
    os.makedirs(dest, exist_ok=True)

    class Handler(BaseHTTPRequestHandler):
        def do_PUT(self):  # noqa: N802
            name = os.path.basename(self.path.split("?")[0]) or "upload.bin"
            target = os.path.join(dest, name)
            length = int(self.headers.get("Content-Length") or 0)
            written = 0
            with open(target, "wb") as handle:
                while written < length:
                    chunk = self.rfile.read(min(1 << 20, length - written))
                    if not chunk:
                        break
                    handle.write(chunk)
                    written += len(chunk)
            print(f"PUT {name} {written}/{length}", flush=True)
            self.send_response(201 if written == length else 500)
            self.end_headers()
            self.wfile.write(str(written).encode())

        def do_GET(self):  # noqa: N802
            self.send_response(200)
            self.end_headers()
            self.wfile.write(b"ok")

        def log_message(self, format, *args):  # noqa: A002 - keep to our own PUT lines
            pass

    ThreadingHTTPServer(("0.0.0.0", port), Handler).serve_forever()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
