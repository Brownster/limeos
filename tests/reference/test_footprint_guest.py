"""Figure extraction and gates of the footprint guest suite, from synthetic measure.sh reports.

No guest, systemd or measure.sh run: each case builds report.json and raw.txt as measure.sh
writes them for one idle window, with the systemctl states around it, and checks what
window_figures and evaluate make of them.
"""

import copy
import importlib.util
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location("footprint_guest", ROOT / "tests/privileged_vm/footprint_guest.py")
guest = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(guest)

MIB = 1024 * 1024
CLK = 100
ELAPSED = 200.0
BASE = ["limeos-storaged.service", "limeos-containerd.service", "limeos-core.service"]
READER = "limeos-storage-reader.service"
# unit: (main pid, PSS bytes, CPU ticks over the window, bytes written over the window)
HEALTHY = {
    "limeos-storaged.service": (101, int(1.6 * MIB), 2, 0),
    "limeos-containerd.service": (102, int(1.6 * MIB), 0, 0),
    "limeos-core.service": (103, int(10.2 * MIB), 14, 16384),
    READER: (104, int(1.9 * MIB), 24, 0),
}


class Window:
    """One window as measure.sh reports it, built from per-process figures so rows and processes agree."""

    def __init__(self, units=None):
        self.units = copy.deepcopy(units or HEALTHY)
        self.extra = []  # (unit, pid, start ticks or None, end ticks or None, pss or None)
        self.overrides = {}
        self.notes = {}
        self.base = list(BASE)
        self.memory = {u: {"Rss": pss // 1024 + 900, "Pss": pss // 1024, "Pss_Anon": pss // 4096,
                           "Pss_File": pss // 1024 - pss // 4096, "AnonHugePages": 0}
                       for u, (_pid, pss, _t, _w) in self.units.items()}

    def build(self):
        processes, start, end = [], [], []
        for unit, (pid, pss, ticks, written) in self.units.items():
            start.append(f"sample start {unit} {pid} 500 1000 2000")
            end.append(f"sample end {unit} {pid} 500 {1000 + ticks} {2000 + written}")
            processes.append({"unit": unit, "pid": pid, "comm": "limeos-executor", "pss_bytes": pss,
                              "swap_bytes": 0, "swap_pss_bytes": 0, "cpu_ticks_delta": ticks,
                              "write_bytes_delta": written})
        for unit, pid, at_start, at_end, pss in self.extra:
            if at_start is not None:
                start.append(f"sample start {unit} {pid} 900 {at_start} 0")
            if at_end is not None:
                end.append(f"sample end {unit} {pid} 900 {at_end} 0")
            if pss is not None:
                processes.append({"unit": unit, "pid": pid, "comm": "limeos-stack-re", "pss_bytes": pss,
                                  "swap_bytes": 0, "swap_pss_bytes": 0})
        matched_ticks = sum(t for _, _, t, _ in self.units.values())
        written = sum(w for _, _, _, w in self.units.values())
        base_pss = sum(p["pss_bytes"] for p in processes if p["unit"] in self.base)
        rows = {
            "core_base_idle_pss": base_pss,
            "core_base_idle_swap": 0,
            "all_resident_idle_pss": sum(p["pss_bytes"] for p in processes),
            "backend_cpu_idle": round(matched_ticks / CLK / ELAPSED * 100, 4),
            "resident_disk_writes": round(written / ELAPSED * 86400 / 1e6, 3),
            "health_http_p95": None,
        }
        rows.update(self.overrides)
        measurements = []
        for ident, value in rows.items():
            row = {"id": ident, "value": value, "status": "pass"}
            if ident == "backend_cpu_idle":
                row["elapsed_s"] = ELAPSED
            if ident in self.notes:
                row["note"] = self.notes[ident]
            measurements.append(row)
        report = {
            "meta": {"clk_tck": str(CLK), "base_units": " ".join(self.base)},
            "missing_units": [], "unknown_units": [],
            "processes": processes, "measurements": measurements,
        }
        raw = "\n".join(["meta clk_tck 100", *start, *end]) + "\n"
        states = {u: {"MainPID": str(pid), "NRestarts": "0"} for u, (pid, *_rest) in self.units.items()}
        return report, raw, states, copy.deepcopy(states)


def figures(window, before=None, after=None):
    report, raw, b, a = window.build()
    memory = {u: m for u, m in window.memory.items() if u in window.units}
    return guest.window_figures(report, raw, before or b, after or a, memory)


THP_OFF = "always madvise [never]"


def run(windows=None, readiness=(120.0, 130.0, 140.0), thp=THP_OFF):
    windows = windows if windows is not None else [figures(Window()) for _ in range(3)]
    return guest.evaluate(windows, list(readiness), thp)


class HealthyTests(unittest.TestCase):
    def test_a_healthy_run_passes(self):
        result = run()
        self.assertEqual(result["result"], "pass", result["problems"])
        self.assertEqual(result["gates"]["backend_cpu_idle"]["value_percent"], 0.2)
        self.assertFalse(result["gates"]["backend_cpu_idle"]["budget_met"])
        self.assertEqual(result["cpu_by_unit_percent"][READER], 0.12)
        self.assertEqual(result["gates"]["core_restart_readiness"]["value_ms"], 130.0)

    def test_a_helper_of_the_reader_coming_and_going_is_tolerated_and_recorded(self):
        windows = []
        for at_start, at_end, pss in ((7, None, None), (None, 3, 300000), (None, None, None)):
            window = Window()
            if at_start is not None or at_end is not None:
                window.extra.append((READER, 777, at_start, at_end, pss))
            windows.append(figures(window))
        result = run(windows)
        self.assertEqual(result["result"], "pass", result["problems"])
        self.assertEqual(windows[0]["tolerated_reader_helpers"],
                         [{"unit": READER, "pid": 777, "only_at": "start", "cpu_ticks_then": 7}])
        self.assertEqual(windows[1]["tolerated_reader_helpers"][0]["only_at"], "end")

    def test_memory_detail_is_recorded_per_unit(self):
        window = figures(Window())
        self.assertEqual(set(window["memory_kb"]), set(HEALTHY))
        self.assertEqual(window["memory_kb"]["limeos-core.service"]["AnonHugePages"], 0)
        self.assertIn("Pss_File", window["memory_kb"][READER])

    def test_seeding_writes_flushes_as_core_does(self):
        now = 1_800_000_030
        flushes = guest.seeded_flushes(now)
        rows = [row for batch, _ in flushes for row in batch]
        self.assertTrue(all(len(batch) == 5 for batch, _ in flushes))
        times = [row[0] for row in rows]
        self.assertEqual(times, sorted(times))
        self.assertEqual(len(set(times)), len(times))
        self.assertEqual(times[-1], 1_800_000_000 - 60)
        self.assertEqual(len(times), 35 * 1440)
        # Each flush deletes what is 31 days older than its newest sample, as telemetry.rs does.
        for batch, cutoff in flushes:
            self.assertEqual(cutoff, batch[-1][0] - 31 * 86400)
        kept = [t for t in times if t >= flushes[-1][1]]
        # The oldest kept sits on the boundary, as the soak finds (44,641 rows).
        self.assertEqual(len(kept), 31 * 1440 + 1)

    def test_lib_base_units_reads_the_real_library(self):
        lib = (ROOT / "scripts/cutover/lib.sh").read_text()
        self.assertEqual(guest.lib_base_units(lib), BASE)

    def test_the_fields_read_are_the_ones_measure_sh_writes(self):
        measure = (ROOT / "scripts/cutover/measure.sh").read_text()
        for name in ("core_base_idle_pss", "backend_cpu_idle", "resident_disk_writes", "core_restart_readiness",
                     "cpu_ticks_delta", "write_bytes_delta", "pss_bytes", "elapsed_s", "base_units",
                     "missing_units", "unknown_units", "process set changed during the window",
                     "printf 'sample %s %s %s %s\\n'"):
            self.assertIn(name, measure)


class FailureTests(unittest.TestCase):
    def assert_fails(self, result, fragment):
        self.assertEqual(result["result"], "fail")
        text = " ".join(result["problems"] + result["failed_gates"])
        self.assertIn(fragment, text)

    def window_run(self, mutate, fragment):
        windows = []
        for index in range(3):
            window = Window()
            if index == 1:
                mutate(window)
            windows.append(figures(window))
        self.assert_fails(run(windows), fragment)

    def test_reader_absent(self):
        self.window_run(lambda w: w.units.pop(READER), READER)

    def test_base_unit_absent_from_processes(self):
        def drop(window):
            report, raw, before, after = window.build()
            report["processes"] = [p for p in report["processes"] if p["unit"] != "limeos-storaged.service"]
            window.build = lambda: (report, raw, before, after)
        self.window_run(drop, "limeos-storaged.service: main process 101 not in")

    def test_base_unit_with_zero_pss(self):
        def zero(window):
            pid, _pss, ticks, written = window.units["limeos-containerd.service"]
            window.units["limeos-containerd.service"] = (pid, 0, ticks, written)
        self.window_run(zero, "limeos-containerd.service: main process 102 measured without PSS")

    def test_processes_list_empty_with_rows_reporting_load(self):
        # The vacuous pass the earlier suite allowed: measure.sh's rows say 50 % and 432 MB/day,
        # the process list is empty.
        def empty(window):
            window.overrides = {"backend_cpu_idle": 50.0, "resident_disk_writes": 432.0}
            report, raw, before, after = window.build()
            report["processes"] = []
            window.build = lambda: (report, raw, before, after)
        windows = [figures(Window()), figures(Window()), None]
        third = Window()
        empty(third)
        windows[2] = figures(third)
        result = run(windows)
        self.assert_fails(result, "not in measure.sh's processes")
        self.assertIn("backend_cpu_idle", result["failed_gates"])
        self.assertIn("resident_disk_writes", result["failed_gates"])

    def test_processes_empty_rows_zero(self):
        def empty(window):
            report, raw, before, after = window.build()
            report["processes"] = []
            for row in report["measurements"]:
                if row["id"] in ("backend_cpu_idle", "resident_disk_writes"):
                    row["value"] = 0.0
            window.build = lambda: (report, raw, before, after)
        self.window_run(empty, "not in measure.sh's processes")

    def test_a_fatal_note_on_a_row(self):
        def note(window):
            window.notes["resident_disk_writes"] = "write_bytes unreadable (run as root)"
        self.window_run(note, "write_bytes unreadable")

    def test_core_base_idle_pss_none(self):
        def none(window):
            window.overrides = {"core_base_idle_pss": None}
        self.window_run(none, "core_base_idle_pss has no value")

    def test_rows_disagreeing_with_processes(self):
        def disagree(window):
            window.overrides = {"backend_cpu_idle": 0.35}
        self.window_run(disagree, "disagrees with the row")

    def test_a_restart_row_none(self):
        self.assert_fails(run(readiness=(120.0, None, 140.0)), "core_restart_readiness")

    def test_fewer_restarts(self):
        self.assert_fails(run(readiness=(120.0, 140.0)), "core_restart_readiness")

    def test_pss_over_the_gate(self):
        def big(window):
            pid, _pss, ticks, written = window.units["limeos-core.service"]
            window.units["limeos-core.service"] = (pid, 13 * MIB, ticks, written)
        self.window_run(big, "core_base_idle_pss")

    def test_disk_writes_over_the_gate(self):
        units = copy.deepcopy(HEALTHY)
        pid, pss, ticks, _written = units["limeos-core.service"]
        units["limeos-core.service"] = (pid, pss, ticks, 32000)  # about 13.8 MB/day
        self.assert_fails(run([figures(Window(units)) for _ in range(3)]), "resident_disk_writes")

    def test_cpu_over_the_ceiling(self):
        units = copy.deepcopy(HEALTHY)
        pid, pss, _ticks, written = units[READER]
        units[READER] = (pid, pss, 70, written)  # 0.43 % in all
        self.assert_fails(run([figures(Window(units)) for _ in range(3)]), "backend_cpu_idle")

    def test_readiness_over_the_gate(self):
        self.assert_fails(run(readiness=(1200.0, 1100.0, 900.0)), "core_restart_readiness")

    def test_a_helper_of_core_coming_and_going(self):
        self.window_run(lambda w: w.extra.append(("limeos-core.service", 555, None, 4, 100000)),
                        "limeos-core.service pid 555 only at the end")

    def test_the_reader_main_process_replaced(self):
        def replaced(window):
            report, raw, before, after = window.build()
            after[READER] = {"MainPID": "905", "NRestarts": "1"}
            window.build = lambda: (report, raw, before, after)
        self.window_run(replaced, "main process or restart count changed")

    def test_a_restart_counted_with_the_same_main_pid(self):
        def restarted(window):
            report, raw, before, after = window.build()
            after["limeos-core.service"] = {"MainPID": "103", "NRestarts": "1"}
            window.build = lambda: (report, raw, before, after)
        self.window_run(restarted, "limeos-core.service: main process or restart count changed")

    def test_a_missing_unit_in_the_report(self):
        def missing(window):
            report, raw, before, after = window.build()
            report["missing_units"] = ["limeos-core.service"]
            window.build = lambda: (report, raw, before, after)
        self.window_run(missing, "missing units")

    def test_core_idle_for_the_whole_run(self):
        units = copy.deepcopy(HEALTHY)
        pid, pss, _ticks, written = units["limeos-core.service"]
        units["limeos-core.service"] = (pid, pss, 0, written)
        self.assert_fails(run([figures(Window(units)) for _ in range(3)]), "used no CPU over the run")

    def test_core_writing_nothing_for_the_whole_run(self):
        units = copy.deepcopy(HEALTHY)
        pid, pss, ticks, _written = units["limeos-core.service"]
        units["limeos-core.service"] = (pid, pss, ticks, 0)
        self.assert_fails(run([figures(Window(units)) for _ in range(3)]), "wrote nothing over the run")

    def test_too_few_windows(self):
        self.assert_fails(run([figures(Window())]), "1 windows measured")

    def test_no_windows(self):
        self.assert_fails(run([]), "0 windows measured")

    def test_transparent_huge_pages_in_a_unit(self):
        def huge(window):
            window.memory["limeos-core.service"]["AnonHugePages"] = 2048
        self.window_run(huge, "limeos-core.service: 2048 kB of transparent huge pages")

    def test_no_memory_detail_for_a_unit(self):
        self.window_run(lambda w: w.memory.pop(READER), f"{READER}: no smaps_rollup after the window")

    def test_transparent_huge_pages_left_enabled_in_the_guest(self):
        self.assert_fails(run(thp="[always] madvise never"), "transparent huge pages are not disabled")


if __name__ == "__main__":
    unittest.main()
