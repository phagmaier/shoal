# Known flakes and environment sensitivities

Compare the failure's symptom and evidence with these records; a matching test name alone does not establish the same cause. Preserve the failed log, report the failure and rerun results, and investigate differing symptoms or relevant code changes. Do not weaken assertions to hide failures. For a matching intermittent symptom, retry focused, then rerun the full gate.

1. **CLI auto-start flake (T44/T45/T46 family).** `without_systemd_start_spawns_a_detached_daemon`, `workspace_commands_start_the_daemon_when_needed` or `systemd_starts_it_when_it_can` records no fake-systemctl call. CLI/client/testkit sources were unchanged in the observed failures. For this symptom, retry focused, then 20 consecutive repeats, then the full gate. Cause unconfirmed.
2. **Stale-socket broken pipe (T41/T43/T44).** `a_stale_socket_file_is_recovered` closes before `hello` is answered, a first write gets a broken pipe, or the first connect is refused (T49). Passes alone (30/30) and in reruns. Suspected startup race; cause unconfirmed. Keep `TempEnv` logs for failed tests.
3. **Prompt timing under load (T16/T43).** `shoal prompt` tests take ~530 ms under full parallel load, pass alone. Git segment has an 80 ms timeout plus 95 ms budget; load skews it. Retry alone before touching code.
4. **Filesystem sensitivity (T43).** `workspace_json_matches_format_one` and `shutdown_saves_every_pane` fail with `TMPDIR=/var/tmp` (btrfs) including alone, pass on runtime tmpfs. Use `/run/user/$(id -u)/shoal-check`. Do not change daemon persistence code for this.
5. **Synthetic `/tmp/.git` (T43).** This runtime presents a `/tmp/.git` that confuses outside-repository prompt tests. Use runtime tmpfs (see `docs/agent-env.md`).

All flakes above were observed passing on retry without code changes. Record the log path (`target/*check*.log`) in the task record when one hits.
