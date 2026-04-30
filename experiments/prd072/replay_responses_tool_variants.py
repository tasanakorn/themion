#!/usr/bin/env python3
import argparse
import copy
import json
import os
import sys
import time
import urllib.error
import urllib.parse
import urllib.request
import uuid
from pathlib import Path
from typing import Any, Dict, List, Optional, Tuple

TOKEN_URL = "https://auth.openai.com/oauth/token"
CLIENT_ID = "oai-prod-frontend"
CODEX_DEFAULT_BASE_URL = "https://chatgpt.com/backend-api/codex"


def load_json(path: Path) -> Dict[str, Any]:
    return json.loads(path.read_text())


def save_json(path: Path, payload: Dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(payload, indent=2))


def default_auth_path() -> Optional[Path]:
    xdg = os.environ.get("XDG_CONFIG_HOME")
    if xdg:
        return Path(xdg) / "themion" / "auth.json"
    home = os.environ.get("HOME")
    if home:
        return Path(home) / ".config" / "themion" / "auth.json"
    return None


def normalize_text(text: str) -> str:
    return " ".join(text.strip().split())


def trim_suffix_by(text: str, reduce_by: int) -> str:
    text = normalize_text(text)
    if reduce_by <= 0:
        return text
    if len(text) <= reduce_by:
        return ""
    return text[: len(text) - reduce_by]


def apply_global_reduce_by(entries: List[Tuple[Dict[str, Any], str]], reduce_by: Optional[int]) -> None:
    if not reduce_by or reduce_by <= 0:
        return
    remaining = reduce_by
    for container, key in reversed(entries):
        if remaining <= 0:
            break
        value = container.get(key)
        if not isinstance(value, str) or not value:
            continue
        current_len = len(value)
        cut = min(current_len, remaining)
        container[key] = trim_suffix_by(value, cut)
        remaining -= cut


def mutate_tools(
    tools: List[Dict[str, Any]],
    global_description_reduce_by: Optional[int],
    global_property_reduce_by: Optional[int],
) -> List[Dict[str, Any]]:
    cloned = copy.deepcopy(tools)
    description_entries: List[Tuple[Dict[str, Any], str]] = []
    property_entries: List[Tuple[Dict[str, Any], str]] = []

    for tool in cloned:
        if isinstance(tool.get("description"), str):
            description_entries.append((tool, "description"))
        properties = tool.get("parameters", {}).get("properties", {})
        for spec in properties.values():
            if isinstance(spec, dict) and isinstance(spec.get("description"), str):
                property_entries.append((spec, "description"))

    apply_global_reduce_by(description_entries, global_description_reduce_by)
    apply_global_reduce_by(property_entries, global_property_reduce_by)
    return cloned


def inject_nonce(request_payload: Dict[str, Any], variant_name: str, repeat_index: int) -> str:
    nonce = f"cache-bust:{variant_name}:r{repeat_index}:{uuid.uuid4()}"
    req = request_payload
    req["instructions"] = (
        f"Nonce for cache-busting only: {nonce}\n\n" + (req.get("instructions") or "")
    )
    return nonce


def build_variant(
    base_request: Dict[str, Any], settings: Dict[str, Any], variant_name: str, repeat_index: int
) -> Tuple[Dict[str, Any], str]:
    req = copy.deepcopy(base_request)
    req["stream"] = True
    if settings.get("include_tools", True):
        req["tools"] = mutate_tools(
            req.get("tools", []),
            global_description_reduce_by=settings.get("global_description_reduce_by"),
            global_property_reduce_by=settings.get("global_property_reduce_by"),
        )
    else:
        req.pop("tools", None)
    nonce = inject_nonce(req, variant_name, repeat_index)
    return req, nonce


def estimate_tool_chars(req: Dict[str, Any]) -> Tuple[int, int]:
    tools = req.get("tools", [])
    desc_chars = 0
    prop_desc_chars = 0
    for tool in tools:
        desc_chars += len(tool.get("description", "") or "")
        for spec in tool.get("parameters", {}).get("properties", {}).values():
            if isinstance(spec, dict):
                prop_desc_chars += len(spec.get("description", "") or "")
    return desc_chars, prop_desc_chars


def refresh_codex_auth(auth: Dict[str, Any]) -> Dict[str, Any]:
    refresh_token = auth["refresh_token"]
    body = urllib.parse.urlencode(
        {
            "grant_type": "refresh_token",
            "refresh_token": refresh_token,
            "client_id": CLIENT_ID,
        }
    ).encode("utf-8")
    req = urllib.request.Request(
        TOKEN_URL,
        data=body,
        method="POST",
        headers={"Content-Type": "application/x-www-form-urlencoded"},
    )
    with urllib.request.urlopen(req, timeout=60) as resp:
        payload = json.loads(resp.read().decode("utf-8"))
    now = int(time.time())
    auth["access_token"] = payload["access_token"]
    auth["refresh_token"] = payload.get("refresh_token", refresh_token)
    auth["expires_at"] = now + int(payload.get("expires_in", 3600))
    return auth


def ensure_fresh_auth(auth: Dict[str, Any], auth_path: Optional[Path]) -> Dict[str, Any]:
    expires_at = int(auth.get("expires_at", 0) or 0)
    if expires_at > int(time.time()) + 60:
        return auth
    auth = refresh_codex_auth(dict(auth))
    if auth_path is not None:
        save_json(auth_path, auth)
    return auth


def consume_sse_json_response(resp: Any) -> Tuple[Dict[str, Any], List[str]]:
    final_response = None
    events: List[str] = []
    for raw_line in resp:
        line = raw_line.decode("utf-8", errors="replace").strip()
        if not line:
            continue
        if line.startswith("event: "):
            events.append(line[len("event: ") :])
            continue
        if not line.startswith("data: "):
            continue
        payload = line[len("data: ") :]
        if payload == "[DONE]":
            break
        try:
            data = json.loads(payload)
        except json.JSONDecodeError:
            continue
        if isinstance(data, dict) and "response" in data and isinstance(data["response"], dict):
            final_response = data["response"]
        elif isinstance(data, dict) and data.get("type") == "response":
            final_response = data
    if final_response is None:
        raise RuntimeError("no final response object found in SSE stream")
    return final_response, events


def call_codex_responses(
    request_payload: Dict[str, Any],
    auth: Dict[str, Any],
    base_url: str,
    timeout_s: int,
) -> Tuple[Dict[str, Any], Dict[str, str], int, List[str]]:
    body = json.dumps(request_payload).encode("utf-8")
    url = base_url.rstrip("/") + "/responses"
    req = urllib.request.Request(
        url,
        data=body,
        method="POST",
        headers={
            "Authorization": f"Bearer {auth['access_token']}",
            "chatgpt-account-id": auth["account_id"],
            "originator": "pi",
            "OpenAI-Beta": "responses=experimental",
            "Accept": "text/event-stream",
            "Content-Type": "application/json",
        },
    )
    with urllib.request.urlopen(req, timeout=timeout_s) as resp:
        payload, events = consume_sse_json_response(resp)
        headers = {k.lower(): v for k, v in resp.headers.items()}
        return payload, headers, resp.status, events


def extract_usage(resp: Dict[str, Any]) -> Dict[str, Any]:
    usage = resp.get("usage") or {}
    return {
        "input_tokens": usage.get("input_tokens"),
        "output_tokens": usage.get("output_tokens"),
        "total_tokens": usage.get("total_tokens"),
        "input_tokens_details": usage.get("input_tokens_details"),
        "raw_usage": usage,
    }


def default_variants() -> List[Tuple[str, Dict[str, Any]]]:
    variants: List[Tuple[str, Dict[str, Any]]] = [
        ("baseline", {"include_tools": True}),
        ("no_tools", {"include_tools": False}),
    ]
    for n in (32, 64, 128, 256):
        variants.append(
            (f"global_desc_reduce_by_{n}", {"include_tools": True, "global_description_reduce_by": n})
        )
        variants.append(
            (
                f"global_desc_and_param_reduce_by_{n}",
                {
                    "include_tools": True,
                    "global_description_reduce_by": n,
                    "global_property_reduce_by": n,
                },
            )
        )
    return variants


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Replay a recorded Codex Responses request with tool-schema variants using Themion's auth flow."
    )
    parser.add_argument("round_json", help="Path to recorded round_*.json")
    parser.add_argument(
        "--output",
        default="tmp/api_token_variants_report.json",
        help="Where to write the comparison report",
    )
    parser.add_argument(
        "--base-url",
        default=CODEX_DEFAULT_BASE_URL,
        help="Codex Responses API base URL",
    )
    parser.add_argument(
        "--auth-path",
        default=None,
        help="Path to Themion Codex auth.json; defaults to ~/.config/themion/auth.json",
    )
    parser.add_argument(
        "--timeout",
        type=int,
        default=120,
        help="HTTP timeout in seconds",
    )
    parser.add_argument(
        "--repeats",
        type=int,
        default=1,
        help="How many times to run each variant with a unique cache-busting nonce",
    )
    args = parser.parse_args()

    auth_path = Path(args.auth_path) if args.auth_path else default_auth_path()
    if auth_path is None or not auth_path.exists():
        print(
            "missing Themion Codex auth.json; pass --auth-path or log in with Themion first",
            file=sys.stderr,
        )
        return 2

    round_obj = load_json(Path(args.round_json))
    base_request = round_obj["request"]
    auth = ensure_fresh_auth(load_json(auth_path), auth_path)

    report = {
        "source_round": args.round_json,
        "provider": round_obj.get("provider"),
        "backend": round_obj.get("backend"),
        "model": round_obj.get("model"),
        "base_usage_from_recording": round_obj.get("meta", {}).get("usage"),
        "base_url": args.base_url,
        "auth_path": str(auth_path),
        "repeats": args.repeats,
        "generated_at_ms": int(time.time() * 1000),
        "variants": [],
    }

    for name, settings in default_variants():
        runs = []
        for repeat_index in range(args.repeats):
            request_payload, nonce = build_variant(base_request, settings, name, repeat_index)
            tool_desc_chars, prop_desc_chars = estimate_tool_chars(request_payload)
            started = time.time()
            try:
                response_payload, headers, status, events = call_codex_responses(
                    request_payload, auth, args.base_url, args.timeout
                )
                run = {
                    "repeat_index": repeat_index,
                    "nonce": nonce,
                    "status": "ok",
                    "http_status": status,
                    "tool_count": len(request_payload.get("tools", [])),
                    "tool_description_chars": tool_desc_chars,
                    "property_description_chars": prop_desc_chars,
                    "duration_ms": int((time.time() - started) * 1000),
                    "usage_summary": extract_usage(response_payload),
                    "response_id": response_payload.get("id"),
                    "seen_events": events,
                    "rate_limit_headers": {
                        k: v
                        for k, v in headers.items()
                        if "codex" in k or "ratelimit" in k or "credit" in k
                    },
                }
            except urllib.error.HTTPError as exc:
                body = exc.read().decode("utf-8", errors="replace")
                run = {
                    "repeat_index": repeat_index,
                    "nonce": nonce,
                    "status": "http_error",
                    "http_status": exc.code,
                    "tool_count": len(request_payload.get("tools", [])),
                    "tool_description_chars": tool_desc_chars,
                    "property_description_chars": prop_desc_chars,
                    "duration_ms": int((time.time() - started) * 1000),
                    "error": body,
                }
            except Exception as exc:
                run = {
                    "repeat_index": repeat_index,
                    "nonce": nonce,
                    "status": "error",
                    "tool_count": len(request_payload.get("tools", [])),
                    "tool_description_chars": tool_desc_chars,
                    "property_description_chars": prop_desc_chars,
                    "duration_ms": int((time.time() - started) * 1000),
                    "error": repr(exc),
                }
            runs.append(run)

        ok_runs = [r for r in runs if r.get("status") == "ok"]
        input_tokens = [r["usage_summary"].get("input_tokens") for r in ok_runs if r.get("usage_summary")]
        cached_tokens = [
            ((r.get("usage_summary") or {}).get("input_tokens_details") or {}).get("cached_tokens")
            for r in ok_runs
        ]
        report["variants"].append(
            {
                "name": name,
                "settings": settings,
                "runs": runs,
                "summary": {
                    "ok_runs": len(ok_runs),
                    "input_tokens": input_tokens,
                    "cached_tokens": cached_tokens,
                    "min_input_tokens": min(input_tokens) if input_tokens else None,
                    "max_input_tokens": max(input_tokens) if input_tokens else None,
                    "avg_input_tokens": (
                        sum(input_tokens) / len(input_tokens) if input_tokens else None
                    ),
                },
            }
        )

    save_json(Path(args.output), report)
    print(json.dumps(report, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
