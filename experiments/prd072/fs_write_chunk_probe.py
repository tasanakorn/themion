#!/usr/bin/env python3
import argparse
import json
import os
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
DEFAULT_MODELS = ["gpt-5.4", "gpt-5.5"]
DEFAULT_SIZES = [2000, 4000, 8000]


def load_json(path: Path) -> Dict[str, Any]:
    return json.loads(path.read_text())


def save_json(path: Path, payload: Dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(payload, indent=2, ensure_ascii=False))


def save_text(path: Path, content: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(content)


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


def make_expected_content(size: int) -> str:
    prefix = f"BEGIN-SIZE-{size}|"
    alphabet = "0123456789abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ-_"
    parts = [prefix]
    while sum(len(part) for part in parts) < size:
        parts.append(alphabet)
    content = "".join(parts)
    return content[:size]


def build_request(model: str, size: int, expected_path: str, expected_content: str, tool_choice: str) -> Dict[str, Any]:
    nonce = f"fs-write-probe:{model}:{size}:{uuid.uuid4()}"
    instructions = (
        "You must produce exactly one function call and no assistant text. "
        "Use the fs_write function exactly once. "
        "Set the path exactly as requested. "
        "Set the content exactly to the provided payload with no omissions, summaries, escaping changes, or extra characters. "
        "Do not ask questions. Do not explain."
    )
    user_text = (
        f"Nonce: {nonce}\n"
        f"Call fs_write once.\n"
        f"path: {expected_path}\n"
        f"content_length: {size}\n"
        "Use this exact content payload below, copied byte-for-byte into the tool argument named content:\n"
        "<PAYLOAD>\n"
        f"{expected_content}\n"
        "</PAYLOAD>"
    )
    request_payload: Dict[str, Any] = {
        "model": model,
        "store": False,
        "stream": True,
        "instructions": instructions,
        "input": [
            {
                "role": "user",
                "content": [
                    {
                        "type": "input_text",
                        "text": user_text,
                    }
                ],
            }
        ],
        "tools": [
            {
                "type": "function",
                "name": "fs_write",
                "description": "Write one file by path with exact content.",
                "parameters": {
                    "type": "object",
                    "additionalProperties": False,
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "Target file path.",
                        },
                        "content": {
                            "type": "string",
                            "description": "Exact file content to write.",
                        },
                    },
                    "required": ["path", "content"],
                },
            }
        ],
    }
    if tool_choice != "none":
        request_payload["tool_choice"] = tool_choice
    return request_payload


def call_codex_responses(
    request_payload: Dict[str, Any],
    auth: Dict[str, Any],
    base_url: str,
    timeout_s: int,
) -> Tuple[List[Dict[str, Any]], Dict[str, str], int]:
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
    records: List[Dict[str, Any]] = []
    with urllib.request.urlopen(req, timeout=timeout_s) as resp:
        for raw_line in resp:
            line = raw_line.decode("utf-8", errors="replace").rstrip("\n")
            records.append({"line": line})
        headers = {k.lower(): v for k, v in resp.headers.items()}
        return records, headers, resp.status


def parse_sse(records: List[Dict[str, Any]]) -> Dict[str, Any]:
    events: List[Dict[str, Any]] = []
    current_event: Optional[str] = None
    data_lines: List[str] = []
    for record in records:
        line = record["line"]
        if not line:
            if current_event is not None or data_lines:
                data_text = "\n".join(data_lines)
                parsed_json = None
                if data_text and data_text != "[DONE]":
                    try:
                        parsed_json = json.loads(data_text)
                    except json.JSONDecodeError:
                        parsed_json = None
                events.append(
                    {
                        "event": current_event,
                        "data_text": data_text,
                        "json": parsed_json,
                    }
                )
            current_event = None
            data_lines = []
            continue
        if line.startswith("event: "):
            current_event = line[len("event: ") :]
        elif line.startswith("data: "):
            data_lines.append(line[len("data: ") :])
    if current_event is not None or data_lines:
        data_text = "\n".join(data_lines)
        parsed_json = None
        if data_text and data_text != "[DONE]":
            try:
                parsed_json = json.loads(data_text)
            except json.JSONDecodeError:
                parsed_json = None
        events.append(
            {
                "event": current_event,
                "data_text": data_text,
                "json": parsed_json,
            }
        )
    return {"events": events}


def analyze_events(parsed: Dict[str, Any]) -> Dict[str, Any]:
    item_names: Dict[str, str] = {}
    item_calls: Dict[str, str] = {}
    arguments_by_item_id: Dict[str, List[str]] = {}
    final_response = None
    event_names: List[str] = []

    for event in parsed["events"]:
        event_name = event.get("event")
        if event_name:
            event_names.append(event_name)
        payload = event.get("json")
        if not isinstance(payload, dict):
            continue

        item = payload.get("item")
        if isinstance(item, dict) and item.get("type") == "function_call":
            item_id = item.get("id")
            if isinstance(item_id, str):
                if isinstance(item.get("name"), str):
                    item_names[item_id] = item["name"]
                if isinstance(item.get("call_id"), str):
                    item_calls[item_id] = item["call_id"]
                arguments_by_item_id.setdefault(item_id, [])

        if isinstance(payload.get("item_id"), str) and isinstance(payload.get("delta"), str):
            item_id = payload["item_id"]
            arguments_by_item_id.setdefault(item_id, []).append(payload["delta"])

        response_obj = payload.get("response")
        if isinstance(response_obj, dict):
            final_response = response_obj
        elif payload.get("type") == "response":
            final_response = payload

    assembled_calls = []
    for item_id, deltas in arguments_by_item_id.items():
        assembled = "".join(deltas)
        parsed_args = None
        parse_error = None
        try:
            parsed_args = json.loads(assembled)
        except json.JSONDecodeError as exc:
            parse_error = str(exc)
        assembled_calls.append(
            {
                "item_id": item_id,
                "call_id": item_calls.get(item_id),
                "name": item_names.get(item_id),
                "delta_count": len(deltas),
                "delta_lengths": [len(chunk) for chunk in deltas],
                "assembled_arguments": assembled,
                "assembled_arguments_length": len(assembled),
                "parsed_arguments": parsed_args,
                "parse_error": parse_error,
            }
        )

    final_function_calls = []
    if isinstance(final_response, dict):
        for item in final_response.get("output", []) or []:
            if not isinstance(item, dict) or item.get("type") != "function_call":
                continue
            final_function_calls.append(
                {
                    "item_id": item.get("id"),
                    "call_id": item.get("call_id"),
                    "name": item.get("name"),
                    "arguments": item.get("arguments"),
                }
            )

    return {
        "event_names": event_names,
        "assembled_calls": assembled_calls,
        "final_response": final_response,
        "final_function_calls": final_function_calls,
    }


def evaluate_result(
    analysis: Dict[str, Any],
    expected_path: str,
    expected_content: str,
) -> Dict[str, Any]:
    selected_call = analysis["assembled_calls"][0] if analysis["assembled_calls"] else None
    parsed_args = selected_call.get("parsed_arguments") if selected_call else None
    actual_path = parsed_args.get("path") if isinstance(parsed_args, dict) else None
    actual_content = parsed_args.get("content") if isinstance(parsed_args, dict) else None
    final_call = analysis["final_function_calls"][0] if analysis["final_function_calls"] else None
    final_arguments = final_call.get("arguments") if isinstance(final_call, dict) else None

    path_match = actual_path == expected_path
    content_match = actual_content == expected_content
    actual_length = len(actual_content) if isinstance(actual_content, str) else None
    expected_length = len(expected_content)
    prefix_match = (
        isinstance(actual_content, str) and actual_content == expected_content[: len(actual_content)]
    )
    assembled_equals_final = final_arguments == selected_call.get("assembled_arguments") if selected_call else False

    if path_match and content_match:
        verdict = "exact_match"
    elif path_match and prefix_match and actual_length is not None and actual_length < expected_length:
        verdict = "likely_truncated_prefix"
    elif actual_content is None:
        verdict = "missing_content_argument"
    elif actual_path is None:
        verdict = "missing_path_argument"
    else:
        verdict = "mismatch"

    return {
        "verdict": verdict,
        "expected_length": expected_length,
        "actual_length": actual_length,
        "path_match": path_match,
        "content_match": content_match,
        "content_prefix_match": prefix_match,
        "assembled_equals_final_arguments": assembled_equals_final,
        "actual_path": actual_path,
        "selected_call_name": selected_call.get("name") if selected_call else None,
        "selected_call_delta_count": selected_call.get("delta_count") if selected_call else None,
        "selected_call_delta_lengths": selected_call.get("delta_lengths") if selected_call else None,
        "final_call_name": final_call.get("name") if isinstance(final_call, dict) else None,
        "final_arguments_length": len(final_arguments) if isinstance(final_arguments, str) else None,
    }


def build_conclusion(summary_rows: List[Dict[str, Any]]) -> str:
    if not summary_rows:
        return "No runs executed."
    exact = [row for row in summary_rows if row.get("verdict") == "exact_match"]
    truncated = [row for row in summary_rows if row.get("verdict") == "likely_truncated_prefix"]
    mismatches = [row for row in summary_rows if row.get("verdict") not in {"exact_match", "likely_truncated_prefix"}]
    if len(exact) == len(summary_rows):
        return (
            "All runs produced an exact fs_write argument match. "
            "This does not support a local SSE de-fragmentation truncation bug for the tested models and payload sizes."
        )
    if truncated and not mismatches:
        return (
            "All non-exact runs were clean prefixes of the expected content. "
            "That pattern is consistent with truncation somewhere in provider generation or stream assembly."
        )
    return (
        "The runs produced mixed outcomes. Inspect the per-run raw SSE logs and assembled argument strings to separate provider behavior from local reassembly issues."
    )


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Probe Codex fs_write function-call argument chunking for 2000/4000/8000-character payloads."
    )
    parser.add_argument(
        "--models",
        default=",".join(DEFAULT_MODELS),
        help="Comma-separated model list. Default: gpt-5.4,gpt-5.5",
    )
    parser.add_argument(
        "--sizes",
        default=",".join(str(size) for size in DEFAULT_SIZES),
        help="Comma-separated content sizes. Default: 2000,4000,8000",
    )
    parser.add_argument(
        "--repeats",
        type=int,
        default=1,
        help="How many times to run each model/size pair.",
    )
    parser.add_argument(
        "--tool-choice",
        default="required",
        help="Value for request.tool_choice. Use 'required' by default or 'none' to omit it.",
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
        default=180,
        help="HTTP timeout in seconds",
    )
    parser.add_argument(
        "--output-dir",
        default="tmp/fs_write_chunk_probe",
        help="Directory for JSON report and raw SSE logs",
    )
    args = parser.parse_args()

    auth_path = Path(args.auth_path) if args.auth_path else default_auth_path()
    if auth_path is None or not auth_path.exists():
        raise SystemExit("missing Themion Codex auth.json; pass --auth-path or log in with Themion first")

    models = [part.strip() for part in args.models.split(",") if part.strip()]
    sizes = [int(part.strip()) for part in args.sizes.split(",") if part.strip()]
    auth = ensure_fresh_auth(load_json(auth_path), auth_path)
    output_dir = Path(args.output_dir)
    output_dir.mkdir(parents=True, exist_ok=True)

    report: Dict[str, Any] = {
        "generated_at_ms": int(time.time() * 1000),
        "base_url": args.base_url,
        "models": models,
        "sizes": sizes,
        "repeats": args.repeats,
        "tool_choice": args.tool_choice,
        "runs": [],
        "summary": [],
    }

    for model in models:
        for size in sizes:
            for repeat_index in range(args.repeats):
                expected_path = f"probe/{model}/size-{size}-repeat-{repeat_index}.txt"
                expected_content = make_expected_content(size)
                request_payload = build_request(
                    model=model,
                    size=size,
                    expected_path=expected_path,
                    expected_content=expected_content,
                    tool_choice=args.tool_choice,
                )
                run_id = f"{model}-size{size}-repeat{repeat_index}"
                sse_log_path = output_dir / f"{run_id}.sse.txt"
                request_path = output_dir / f"{run_id}.request.json"
                save_json(request_path, request_payload)
                started = time.time()

                run_record: Dict[str, Any] = {
                    "run_id": run_id,
                    "model": model,
                    "size": size,
                    "repeat_index": repeat_index,
                    "request_path": str(request_path),
                    "sse_log_path": str(sse_log_path),
                    "expected_path": expected_path,
                    "expected_content_length": len(expected_content),
                }

                try:
                    sse_records, headers, status = call_codex_responses(
                        request_payload, auth, args.base_url, args.timeout
                    )
                    save_text(sse_log_path, "\n".join(record["line"] for record in sse_records) + "\n")
                    parsed = parse_sse(sse_records)
                    analysis = analyze_events(parsed)
                    evaluation = evaluate_result(analysis, expected_path, expected_content)
                    run_record.update(
                        {
                            "status": "ok",
                            "http_status": status,
                            "duration_ms": int((time.time() - started) * 1000),
                            "rate_limit_headers": {
                                k: v
                                for k, v in headers.items()
                                if "codex" in k or "ratelimit" in k or "credit" in k
                            },
                            "event_names": analysis["event_names"],
                            "assembled_calls": analysis["assembled_calls"],
                            "final_function_calls": analysis["final_function_calls"],
                            "evaluation": evaluation,
                        }
                    )
                except urllib.error.HTTPError as exc:
                    body = exc.read().decode("utf-8", errors="replace")
                    save_text(sse_log_path, body)
                    run_record.update(
                        {
                            "status": "http_error",
                            "http_status": exc.code,
                            "duration_ms": int((time.time() - started) * 1000),
                            "error": body,
                        }
                    )
                except Exception as exc:
                    run_record.update(
                        {
                            "status": "error",
                            "duration_ms": int((time.time() - started) * 1000),
                            "error": repr(exc),
                        }
                    )

                report["runs"].append(run_record)
                summary_row = {
                    "run_id": run_id,
                    "model": model,
                    "size": size,
                    "repeat_index": repeat_index,
                    "status": run_record["status"],
                }
                if run_record["status"] == "ok":
                    summary_row.update(run_record["evaluation"])
                else:
                    summary_row["error"] = run_record.get("error")
                report["summary"].append(summary_row)

    report["conclusion"] = build_conclusion(
        [row for row in report["summary"] if row.get("status") == "ok"]
    )

    report_path = output_dir / "report.json"
    save_json(report_path, report)
    print(json.dumps(report, indent=2, ensure_ascii=False))
    print(f"\nReport written to {report_path}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
