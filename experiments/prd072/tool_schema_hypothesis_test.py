#!/usr/bin/env python3
import argparse
import copy
import json
import os
import time
import urllib.error
import urllib.parse
import urllib.request
import uuid
from pathlib import Path
from typing import Any, Dict, List, Optional, Tuple

import tiktoken

TOKEN_URL = "https://auth.openai.com/oauth/token"
CLIENT_ID = "oai-prod-frontend"
CODEX_DEFAULT_BASE_URL = "https://chatgpt.com/backend-api/codex"
ENCODING_NAME = "o200k_base"


def load_round(path: Path) -> Dict[str, Any]:
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


def inject_nonce(req: Dict[str, Any], variant_name: str, repeat_index: int) -> str:
    nonce = f"schema-hyp:{variant_name}:r{repeat_index}:{uuid.uuid4()}"
    req["instructions"] = f"Nonce for cache-busting only: {nonce}\n\n" + (req.get("instructions") or "")
    return nonce


def extract_usage(resp: Dict[str, Any]) -> Dict[str, Any]:
    usage = resp.get("usage") or {}
    return {
        "input_tokens": usage.get("input_tokens"),
        "output_tokens": usage.get("output_tokens"),
        "total_tokens": usage.get("total_tokens"),
        "input_tokens_details": usage.get("input_tokens_details"),
        "raw_usage": usage,
    }


def shorten_tool_names(tools: List[Dict[str, Any]]) -> List[Dict[str, Any]]:
    out = copy.deepcopy(tools)
    for i, tool in enumerate(out):
        tool["name"] = f"t{i}"
        params = tool.get("parameters", {})
        props = params.get("properties", {})
        new_props = {}
        rename_map = {}
        for j, (key, value) in enumerate(props.items()):
            nk = f"p{j}"
            rename_map[key] = nk
            new_props[nk] = value
        params["properties"] = new_props
        if isinstance(params.get("required"), list):
            params["required"] = [rename_map.get(k, k) for k in params["required"]]
    return out


def remove_all_descriptions(tools: List[Dict[str, Any]]) -> List[Dict[str, Any]]:
    out = copy.deepcopy(tools)
    for tool in out:
        tool.pop("description", None)
        props = tool.get("parameters", {}).get("properties", {})
        for spec in props.values():
            if isinstance(spec, dict):
                spec.pop("description", None)
    return out


def descriptions_only_minimal_structure(tools: List[Dict[str, Any]]) -> List[Dict[str, Any]]:
    out = []
    for i, tool in enumerate(tools):
        out.append(
            {
                "type": "function",
                "name": tool.get("name", f"tool_{i}"),
                "description": tool.get("description", ""),
                "parameters": {
                    "type": "object",
                    "properties": {},
                    "required": [],
                },
            }
        )
    return out


def compact_structure_keep_text(tools: List[Dict[str, Any]]) -> List[Dict[str, Any]]:
    out = copy.deepcopy(tools)
    for tool in out:
        params = tool.get("parameters", {})
        props = params.get("properties", {})
        for spec in props.values():
            if not isinstance(spec, dict):
                continue
            keep = {}
            if "type" in spec:
                keep["type"] = spec["type"]
            if "enum" in spec:
                keep["enum"] = spec["enum"]
            if "description" in spec:
                keep["description"] = spec["description"]
            spec.clear()
            spec.update(keep)
    return out


def build_variants(original_tools: List[Dict[str, Any]]) -> List[Tuple[str, List[Dict[str, Any]]]]:
    return [
        ("baseline_tools", copy.deepcopy(original_tools)),
        ("no_tools", []),
        ("no_descriptions", remove_all_descriptions(original_tools)),
        ("descriptions_only_minimal_structure", descriptions_only_minimal_structure(original_tools)),
        ("compact_structure_keep_text", compact_structure_keep_text(original_tools)),
        ("short_names_keep_descriptions", shorten_tool_names(original_tools)),
    ]


def tool_stats(tools: List[Dict[str, Any]], enc: Any) -> Dict[str, Any]:
    raw = json.dumps(tools, separators=(",", ":"), ensure_ascii=False)
    desc_chars = 0
    param_desc_chars = 0
    names_chars = 0
    param_name_chars = 0
    for tool in tools:
        names_chars += len(tool.get("name", "") or "")
        desc_chars += len(tool.get("description", "") or "")
        props = tool.get("parameters", {}).get("properties", {})
        for key, spec in props.items():
            param_name_chars += len(key)
            if isinstance(spec, dict):
                param_desc_chars += len(spec.get("description", "") or "")
    return {
        "tool_count": len(tools),
        "json_chars": len(raw),
        "json_tokens_o200k": len(enc.encode(raw)),
        "tool_name_chars": names_chars,
        "tool_description_chars": desc_chars,
        "param_name_chars": param_name_chars,
        "param_description_chars": param_desc_chars,
    }


def main() -> int:
    parser = argparse.ArgumentParser(description="Synthetic Codex tool-schema hypothesis test")
    parser.add_argument("round_json")
    parser.add_argument("--output", default="tmp/tool_schema_hypothesis_report.json")
    parser.add_argument("--base-url", default=CODEX_DEFAULT_BASE_URL)
    parser.add_argument("--auth-path", default=None)
    parser.add_argument("--timeout", type=int, default=120)
    parser.add_argument("--repeats", type=int, default=2)
    args = parser.parse_args()

    auth_path = Path(args.auth_path) if args.auth_path else default_auth_path()
    if auth_path is None or not auth_path.exists():
        raise SystemExit("missing Themion Codex auth.json")

    round_obj = load_round(Path(args.round_json))
    base_request = round_obj["request"]
    original_tools = copy.deepcopy(base_request["tools"])
    auth = ensure_fresh_auth(json.loads(auth_path.read_text()), auth_path)
    enc = tiktoken.get_encoding(ENCODING_NAME)

    report = {
        "source_round": args.round_json,
        "encoding": ENCODING_NAME,
        "repeats": args.repeats,
        "generated_at_ms": int(time.time() * 1000),
        "variants": [],
    }

    for variant_name, tools in build_variants(original_tools):
        runs = []
        for repeat_index in range(args.repeats):
            req = copy.deepcopy(base_request)
            req["stream"] = True
            if tools:
                req["tools"] = copy.deepcopy(tools)
            else:
                req.pop("tools", None)
            nonce = inject_nonce(req, variant_name, repeat_index)
            stats = tool_stats(req.get("tools", []), enc)
            started = time.time()
            try:
                response_payload, headers, status, events = call_codex_responses(
                    req, auth, args.base_url, args.timeout
                )
                run = {
                    "repeat_index": repeat_index,
                    "nonce": nonce,
                    "status": "ok",
                    "http_status": status,
                    "duration_ms": int((time.time() - started) * 1000),
                    "tool_stats": stats,
                    "usage_summary": extract_usage(response_payload),
                    "response_id": response_payload.get("id"),
                    "seen_events": events,
                    "cached_tokens": ((extract_usage(response_payload).get("input_tokens_details") or {}).get("cached_tokens")),
                    "rate_limit_headers": {
                        k: v
                        for k, v in headers.items()
                        if "codex" in k or "ratelimit" in k or "credit" in k
                    },
                }
            except urllib.error.HTTPError as exc:
                run = {
                    "repeat_index": repeat_index,
                    "nonce": nonce,
                    "status": "http_error",
                    "http_status": exc.code,
                    "duration_ms": int((time.time() - started) * 1000),
                    "tool_stats": stats,
                    "error": exc.read().decode("utf-8", errors="replace"),
                }
            runs.append(run)

        ok_runs = [r for r in runs if r["status"] == "ok"]
        input_tokens = [r["usage_summary"]["input_tokens"] for r in ok_runs]
        cached_tokens = [r.get("cached_tokens") for r in ok_runs]
        report["variants"].append(
            {
                "name": variant_name,
                "runs": runs,
                "summary": {
                    "avg_input_tokens": (sum(input_tokens) / len(input_tokens)) if input_tokens else None,
                    "input_tokens": input_tokens,
                    "cached_tokens": cached_tokens,
                    "tool_stats": tool_stats(tools, enc),
                },
            }
        )

    save_json(Path(args.output), report)
    print(json.dumps(report, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
