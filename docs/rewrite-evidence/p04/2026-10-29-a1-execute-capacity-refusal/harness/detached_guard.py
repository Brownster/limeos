# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Keep the existing qualification guard in its own owned process session."""
from pathlib import Path
import runpy
import signal

def interrupted(signum, frame):
    raise KeyboardInterrupt(signum)

signal.signal(signal.SIGTERM, interrupted)
signal.signal(signal.SIGINT, interrupted)
runpy.run_path(str(Path(__file__).parents[1] / 'main-consolidation/guard.py'), run_name='__main__')
