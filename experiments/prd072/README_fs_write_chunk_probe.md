# fs_write chunk probe experiment

This experiment checks whether Codex function-call arguments for a forced `fs_write`-style tool get truncated around 2k–3k characters, and whether any loss looks like upstream model/provider behavior or local SSE argument de-fragmentation.

## What it does

- sends direct Codex `/responses` requests using Themion's saved Codex auth
- exposes exactly one function tool named `fs_write`
- forces tool use with `tool_choice: "required"` by default
- asks the model to call the tool once with exact `path` and exact `content`
- tests content payload sizes `2000`, `4000`, and `8000` by default
- stores the raw SSE stream for every run
- reassembles `response.function_call_arguments.delta` chunks by `item_id`
- compares the reassembled JSON arguments and final response `output[*].arguments` against the expected content
- writes a JSON report with per-run verdicts and one experiment conclusion string

## Files

- script: `experiments/prd072/fs_write_chunk_probe.py`
- output dir default: `tmp/fs_write_chunk_probe/`
  - `*.request.json` = exact request body sent
  - `*.sse.txt` = raw SSE lines returned by Codex
  - `report.json` = structured results and conclusion

## Usage

Run the default matrix:

```bash
python3 experiments/prd072/fs_write_chunk_probe.py
```

Run with repeats for more confidence:

```bash
python3 experiments/prd072/fs_write_chunk_probe.py --repeats 3
```

Test only one model:

```bash
python3 experiments/prd072/fs_write_chunk_probe.py --models gpt-5.4
```

Use a custom auth file or output location:

```bash
python3 experiments/prd072/fs_write_chunk_probe.py \
  --auth-path ~/.config/themion/auth.json \
  --output-dir tmp/fs_write_chunk_probe_run2
```

## How to inspect results

Start with:

```bash
python3 - <<'PY'
import json
from pathlib import Path
report = json.loads(Path('tmp/fs_write_chunk_probe/report.json').read_text())
print(json.dumps(report['summary'], indent=2))
print(report['conclusion'])
PY
```

If a run is suspicious, inspect:

- `assembled_calls[0].delta_lengths`
- `assembled_calls[0].assembled_arguments_length`
- `evaluation.actual_length`
- `evaluation.content_prefix_match`
- matching `*.sse.txt` raw stream

## Reading the verdicts

- `exact_match`: path and content exactly match the expected tool arguments
- `likely_truncated_prefix`: path matched and returned content is a clean prefix of expected content
- `missing_content_argument`: tool JSON parsed but `content` was absent
- `missing_path_argument`: tool JSON parsed but `path` was absent
- `mismatch`: tool call happened but the returned content/path did not match the request

## Interpretation guidance

- If both the reassembled delta stream and final `output[*].arguments` are truncated the same way, the issue is probably upstream of Themion's local delta reassembly.
- If the raw SSE log shows complete deltas but the reassembled argument in local analysis is shorter or malformed, that supports a local de-fragmentation bug.
- If all sizes pass exactly, this experiment does not support the current truncation hypothesis for the tested models and prompt shape.
