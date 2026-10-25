# Performance measurements

M1 spec §12 budgets, measured on the user's machine. A regression of more than 20 % on an automated measure fails the check. T51 adds the remaining rows.

Machine: Intel Core i5-9400 (6 cores), Linux 7.2.8, Omarchy.

## Prompt (M1 spec §8.3)

`just bench-prompt` builds a release `shoal` and runs `shoal prompt --shell bash` 300 times per case with `hyperfine --shell=none` after 20 warm-up runs. It reads the real config and Omarchy theme, as a prompt does. The repository case is 5,000 committed files in 50 directories with one modified file. Neither directory has a runtime marker file.

| Case | Budget (p95) | p95 | Mean | Date |
| --- | --- | --- | --- | --- |
| Outside a repository | < 5 ms | 0.72 ms | 0.66 ms | 2026-10-25 |
| Inside a 5,000-file repository | < 30 ms | 18.92 ms | 17.96 ms | 2026-10-25 |

Not covered by the script: a runtime cache miss costs one `<tool> --version` run. Through a mise shim, which is never cached, that is about 14 ms for `node` and 19 ms for `rustc` on this machine, and it runs alongside `git status`.
