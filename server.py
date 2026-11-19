#!/usr/bin/env python3
"""Small dependency-free HTTP server wrapping the original SQL lineage Rust CLI.

No database, persistent storage, third-party Python dependency or shell execution.
"""
from __future__ import annotations

import json
import logging
import os
from pathlib import Path
import subprocess
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from socketserver import ThreadingMixIn
import threading
import time
from urllib.parse import urlsplit

PORT = int(os.getenv("PORT", "8080"))
MAX_SQL_BYTES = int(os.getenv("MAX_SQL_BYTES", "131072"))
RUST_TIMEOUT = float(os.getenv("RUST_TIMEOUT", "8"))
RUST_BIN = os.getenv("SQL_LINEAGE_BIN", "/usr/local/bin/sql-lineage-rs")
ROOT = Path(__file__).resolve().parent
STATIC = ROOT / "web"
EXAMPLE = ROOT / "engine" / "examples" / "demo.sql"
SLOTS = threading.BoundedSemaphore(value=4)
LOGGER = logging.getLogger("sql-lineage-web")
logging.basicConfig(level=logging.INFO, format="%(asctime)s %(levelname)s %(message)s")

FILES = {
    "/": (STATIC / "index.html", "text/html; charset=utf-8"),
    "/app.js": (STATIC / "app.js", "text/javascript; charset=utf-8"),
    "/style.css": (STATIC / "style.css", "text/css; charset=utf-8"),
    "/favicon.svg": (STATIC / "favicon.svg", "image/svg+xml"),
    "/example.sql": (EXAMPLE, "text/plain; charset=utf-8"),
}


class AppHandler(BaseHTTPRequestHandler):
    server_version = "SQLLineageWeb/1.0"
    sys_version = ""

    def log_message(self, fmt: str, *args: object) -> None:
        LOGGER.info("%s %s", self.address_string(), fmt % args)

    def send_common_headers(self) -> None:
        self.send_header("X-Content-Type-Options", "nosniff")
        self.send_header("X-Frame-Options", "DENY")
        self.send_header("Referrer-Policy", "no-referrer")
        self.send_header("Cross-Origin-Resource-Policy", "same-origin")
        self.send_header("Permissions-Policy", "camera=(), microphone=(), geolocation=()")
        self.send_header(
            "Content-Security-Policy",
            "default-src 'none'; script-src 'self'; style-src 'self'; "
            "img-src 'self' data:; connect-src 'self'; font-src 'self'; "
            "base-uri 'none'; form-action 'none'; frame-ancestors 'none'",
        )

    def respond(self, status: int, payload: bytes, content_type: str, *, cache: str = "no-store") -> None:
        self.send_response(status)
        self.send_header("Content-Type", content_type)
        self.send_header("Content-Length", str(len(payload)))
        self.send_header("Cache-Control", cache)
        self.send_common_headers()
        self.end_headers()
        if self.command != "HEAD":
            self.wfile.write(payload)

    def json_response(self, status: int, value: dict) -> None:
        self.respond(status, json.dumps(value, ensure_ascii=False).encode("utf-8"),
                     "application/json; charset=utf-8")

    def do_HEAD(self) -> None:
        self.do_GET()

    def do_GET(self) -> None:
        path = urlsplit(self.path).path
        if path == "/healthz":
            return self.json_response(200, {"status": "ok"})
        if path == "/api/info":
            return self.json_response(200, {"name": "SQL Lineage Studio", "engine": "Rust",
                                            "max_sql_bytes": MAX_SQL_BYTES, "version": "2.0.0", "features": ["ast", "column_lineage", "impact", "dialects"]})
        if path not in FILES:
            return self.json_response(404, {"error": "Page introuvable"})
        file, content_type = FILES[path]
        try:
            payload = file.read_bytes()
        except OSError:
            LOGGER.exception("Missing static asset: %s", file)
            return self.json_response(500, {"error": "Fichier statique indisponible"})
        self.respond(200, payload, content_type,
                     cache="public, max-age=3600" if path in ("/style.css", "/app.js", "/favicon.svg") else "no-cache")

    def do_POST(self) -> None:
        if urlsplit(self.path).path != "/api/analyze":
            return self.json_response(404, {"error": "Route inconnue"})
        content_type = self.headers.get("Content-Type", "").split(";", 1)[0].strip().lower()
        if content_type != "application/json":
            return self.json_response(415, {"error": "Content-Type application/json requis"})
        try:
            length = int(self.headers.get("Content-Length", "-1"))
        except ValueError:
            return self.json_response(400, {"error": "Content-Length invalide"})
        if length < 0:
            return self.json_response(411, {"error": "Content-Length requis"})
        if length > MAX_SQL_BYTES * 2 + 8192:
            return self.json_response(413, {"error": "Requête trop volumineuse"})
        try:
            data = json.loads(self.rfile.read(length).decode("utf-8"))
        except (UnicodeDecodeError, json.JSONDecodeError):
            return self.json_response(400, {"error": "JSON invalide"})
        if not isinstance(data, dict) or not isinstance(data.get("sql"), str):
            return self.json_response(400, {"error": "Le champ 'sql' doit contenir du texte"})
        sql = data["sql"]
        dialect = data.get("dialect", "generic")
        if dialect not in ("generic", "postgres", "mysql", "sqlite"):
            return self.json_response(400, {"error": "Dialecte SQL inconnu"})
        impact_table = data.get("impact_table")
        if impact_table is not None and (not isinstance(impact_table, str) or
                                         not (1 <= len(impact_table) <= 180) or
                                         any(c not in "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789_.$\"`" for c in impact_table)):
            return self.json_response(400, {"error": "Nom de table d'impact invalide"})
        size = len(sql.encode("utf-8"))
        if size > MAX_SQL_BYTES:
            return self.json_response(413, {"error": f"SQL trop volumineux ({size} octets, limite {MAX_SQL_BYTES})"})
        if not sql.strip():
            return self.json_response(400, {"error": "Colle une requête SQL avant d'analyser"})
        if not SLOTS.acquire(blocking=False):
            return self.json_response(429, {"error": "Serveur occupé : réessaie dans un instant"})
        try:
            self.analyze(sql, dialect, impact_table)
        finally:
            SLOTS.release()

    def analyze(self, sql: str, dialect: str, impact_table: str | None) -> None:
        began = time.perf_counter()
        cmd = [RUST_BIN, "--format", "json", "--dialect", dialect]
        if impact_table:
            cmd.extend(["--impact", impact_table])
        cmd.append("-")
        try:
            # One Rust process per request, instead of two in V1.
            result = subprocess.run(
                cmd, input=sql, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                text=True, encoding="utf-8", timeout=RUST_TIMEOUT, check=False, cwd="/tmp",
            )
            if result.returncode != 0:
                message = (result.stderr or "SQL non pris en charge").splitlines()[0]
                return self.json_response(422, {"error": message.removeprefix("Erreur : ")[:600]})
            parsed = json.loads(result.stdout)
            if not isinstance(parsed, dict) or not isinstance(parsed.get("statements"), list):
                raise ValueError("Format JSON du moteur incorrect")
            statements = parsed["statements"]
            sources, targets, links = set(), set(), set()
            for statement in statements:
                srcs = [s for s in statement.get("sources", []) if isinstance(s, str)]
                sources.update(srcs)
                target = statement.get("target")
                if isinstance(target, str) and target:
                    targets.add(target)
                    links.update((s, target) for s in srcs)
            parsed["stats"] = {
                "statements": len(statements), "sources": len(sources),
                "targets": len(targets), "relations": len(links),
                "column_relations": len(parsed.get("columns", [])),
                "duration_ms": round((time.perf_counter() - began) * 1000, 1),
            }
            return self.json_response(200, parsed)
        except subprocess.TimeoutExpired:
            LOGGER.warning("Rust analysis timeout")
            self.json_response(504, {"error": "Analyse interrompue (durée maximale dépassée)"})
        except (OSError, ValueError, json.JSONDecodeError):
            LOGGER.exception("Internal analysis failure")
            self.json_response(500, {"error": "Erreur interne pendant l'analyse"})


def main() -> None:
    if not (1 <= PORT <= 65535):
        raise SystemExit("PORT invalide")
    if not Path(RUST_BIN).is_file():
        raise SystemExit(f"Binaire Rust introuvable : {RUST_BIN}")
    server = ThreadingHTTPServer(("0.0.0.0", PORT), AppHandler)
    server.daemon_threads = True
    LOGGER.info("SQL Lineage Studio listening on port %s", PORT)
    try:
        server.serve_forever(poll_interval=0.5)
    except KeyboardInterrupt:
        LOGGER.info("Stopping")
    finally:
        server.server_close()


if __name__ == "__main__":
    main()
