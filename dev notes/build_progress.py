"""Writes `dev notes/build_progress.md`: where the build stands (features by phase and status, from the plan and
docs/features.toml), what is outstanding (a hand-kept block the script preserves), and a date log of the work (time
spent, commits and finished work per day).

    python "dev notes/build_progress.py" [TRANSCRIPT.jsonl ...]

Time spent comes from the agent session transcripts given: every message and tool step is time-stamped, and the time
between two steps counts as work when they are under IDLE apart. Without transcripts the log shows commits and work
only.
"""

import re
import subprocess
import sys
import tomllib
from collections import Counter, defaultdict
from datetime import datetime, timedelta
from pathlib import Path
from zoneinfo import ZoneInfo

ROOT = Path(__file__).resolve().parent.parent
OUT = ROOT / "dev notes" / "build_progress.md"
PLAN = ROOT / "dev notes" / "EcclesiaPDF_detailed_plan_nov_9th.md"
ZONE = ZoneInfo("America/New_York")
IDLE = timedelta(minutes=15)
KEEP = ("<!-- outstanding: kept by hand, the script leaves this block as it is -->", "<!-- /outstanding -->")
STAMP = re.compile(rb'"timestamp":"([0-9T:.\-]+)Z"')


def features():
    """(phase, label, tag, status, open) for every plan §27 entry, in plan order."""
    status = {k: v for k, v in tomllib.loads((ROOT / "docs" / "features.toml").read_text("utf-8")).items()
              if isinstance(v, dict)}
    rows = []
    for m in re.finditer(r"^#### (Ph(\d+): \d+/\d+ \| ([A-Z]+\.\d+):? .+?)\s*$", PLAN.read_text("utf-8"), re.M):
        label, phase, tag = m.group(1), int(m.group(2)), m.group(3)
        entry = status.get(tag, {})
        rows.append((phase, label, tag, entry.get("status", "not started"), entry.get("open", "")))
    return rows


def active_time(transcripts):
    """Working time per local date, from the time-stamped steps of each transcript."""
    per_day = defaultdict(timedelta)
    for path in transcripts:
        stamps = []
        with open(path, "rb") as f:
            for line in f:
                m = STAMP.search(line)
                if m:
                    stamps.append(datetime.fromisoformat(m.group(1).decode() + "+00:00").astimezone(ZONE))
        stamps.sort()
        for a, b in zip(stamps, stamps[1:]):
            if b - a <= IDLE:
                per_day[a.date()] += b - a
    return per_day


def commits():
    out = subprocess.run(["git", "-C", str(ROOT), "log", "--format=%aI"], capture_output=True, text=True,
                         check=True).stdout.split()
    return Counter(datetime.fromisoformat(s).astimezone(ZONE).date() for s in out)


def finished_work():
    """The CHANGELOG headings per date, oldest first within a day."""
    per_day = defaultdict(list)
    for m in re.finditer(r"^### (\d{4}-\d{2}-\d{2}) \d{2}:\d{2}[:;] (.+)$", (ROOT / "CHANGELOG.md").read_text("utf-8"),
                         re.M):
        per_day[datetime.strptime(m.group(1), "%Y-%m-%d").date()].insert(0, m.group(2).strip())
    return per_day


def hours(t):
    minutes = round(t.total_seconds() / 60)
    return f"{minutes // 60}:{minutes % 60:02d}"


def kept_block():
    if OUT.exists():
        text = OUT.read_text("utf-8")
        if KEEP[0] in text and KEEP[1] in text:
            return text[text.index(KEEP[0]):text.index(KEEP[1]) + len(KEEP[1])]
    return f"{KEEP[0]}\n\n- (nothing yet)\n\n{KEEP[1]}"


def main(transcripts):
    rows = features()
    time = active_time(transcripts)
    done, work = commits(), finished_work()
    now = datetime.now(ZONE)
    order = ["shipped", "in-progress", "not started"]

    out = [
        "# Build progress",
        "",
        f"Updated {now:%Y-%m-%d %H:%M} by `dev notes/build_progress.py` from the plan, `docs/features.toml`, "
        "`CHANGELOG.md`, git and the agent session transcripts. Only the Outstanding block is edited by hand.",
        "",
        "A feature is **shipped** once the maintainer has accepted it, **in progress** from the first commit until then "
        "(built, tests passing, waiting for acceptance or with open gaps), and **not started** before that.",
        "",
        "## Summary",
        "",
        "| Phase | Total | Shipped | In progress | Not started |",
        "| --- | --- | --- | --- | --- |",
    ]
    by_phase = defaultdict(list)
    for r in rows:
        by_phase[r[0]].append(r)
    totals = Counter()
    for phase in sorted(by_phase):
        c = Counter(r[3] for r in by_phase[phase])
        totals.update(c)
        out.append(f"| Ph{phase} | {len(by_phase[phase])} | " + " | ".join(str(c[s]) for s in order) + " |")
    out.append(f"| **All** | **{len(rows)}** | " + " | ".join(f"**{totals[s]}**" for s in order) + " |")
    days = sorted(set(time) | set(done) | set(work))
    out += [
        "",
        f"Work so far: {hours(sum(time.values(), timedelta()))} hours of agent time over "
        f"{sum(1 for d in days if time.get(d) or work.get(d))} working days; {sum(done.values())} commits; "
        f"{sum(len(v) for v in work.values())} changelog entries.",
        "",
        "## Outstanding",
        "",
        kept_block(),
        "",
        "## Date log",
        "",
        "Newest first. Time is agent working time (steps less than 15 minutes apart); the list is the changelog "
        "entries finished that day.",
        "",
    ]
    for d in reversed(days):
        head = [hours(time[d]) + " hours"] if d in time else []
        head += [f"{done[d]} commits"] if done.get(d) else []
        out += [f"### {d:%Y-%m-%d} ({d:%a})" + (": " + "; ".join(head) if head else ""), ""]
        out += [f"- {w}" for w in work.get(d, [])] or ["- (planning and setup; no changelog entries)"]
        out.append("")
    out += ["## Features by phase", ""]
    for phase in sorted(by_phase):
        c = Counter(r[3] for r in by_phase[phase])
        out += [f"### Phase {phase}: {c['shipped']} shipped; {c['in-progress']} in progress; "
                f"{c['not started']} not started", ""]
        for s in order:
            items = [r for r in by_phase[phase] if r[3] == s]
            if not items:
                continue
            out.append(f"**{s.capitalize().replace('-', ' ')} ({len(items)})**")
            out.append("")
            for _, label, _, _, gaps in items:
                out.append(f"- {label}" + (f" (open: {gaps})" if gaps and s != "not started" else ""))
            out.append("")
    OUT.write_text("\n".join(out).rstrip("\n") + "\n", "utf-8", newline="\n")
    print(OUT)


if __name__ == "__main__":
    main(sys.argv[1:])
