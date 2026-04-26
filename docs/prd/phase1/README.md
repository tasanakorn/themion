# PRD-059 Phase 1 Spike Notes

This directory holds the isolated Phase 1 evaluation notes and references for PRD-059.

## Files

- Evaluation corpus fixture: `experiments/prd059/fixtures/prd-059-semantic-search-corpus.json`

## Spike runner

Run the isolated prototype with:

```bash
rust-script experiments/prd059/phase1_spike.rs
```

Optional flags:

- `--dataset <path>`
- `--artifact-dir <dir>`

The runner writes per-scenario SQLite artifacts plus `summary.json` under the chosen artifact directory.

## Evaluation contract

- The dataset fixture lives outside `docs/` so docs stay human-friendly while the spike remains reproducible.
- Ranking applies project scope plus explicit node-type and hashtag filters before semantic scoring.
