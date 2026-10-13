"""Passive memory/reclaim diagnosis after the guard stopped the owned guest."""
import json
import pathlib
import subprocess
import time
result = {'captured_utc':time.strftime('%Y-%m-%dT%H:%M:%SZ',time.gmtime()), 'meminfo':pathlib.Path('/proc/meminfo').read_text(), 'vmstat':pathlib.Path('/proc/vmstat').read_text(), 'swappiness':pathlib.Path('/proc/sys/vm/swappiness').read_text().strip(), 'processes':[]}
for directory in pathlib.Path('/proc').iterdir():
    if not directory.name.isdigit():
        continue
    try:
        text = (directory/'status').read_text()
        fields = {line.split(':',1)[0]:line.split(':',1)[1].strip() for line in text.splitlines() if ':' in line}
        rss = int(fields.get('VmRSS','0').split()[0])
        swap = int(fields.get('VmSwap','0').split()[0])
        if rss >= 32768 or swap >= 1024:
            stat = (directory/'stat').read_text()
            values = stat[stat.rindex(')')+2:].split()
            result['processes'].append({'pid':int(directory.name),'name':fields['Name'],'rss_kib':rss,'swap_kib':swap,'major_faults':int(values[9]),'start_ticks':int(values[19])})
    except (OSError, ValueError, KeyError):
        pass
result['processes'].sort(key=lambda process:process['swap_kib'],reverse=True)
result['memory_pressure_available'] = pathlib.Path('/proc/pressure/memory').exists()
result['services'] = subprocess.check_output(['systemctl','show','pi-health','pihealth-helper','docker','smbd','--property=Id,ActiveState,SubState,MainPID,NRestarts'],text=True)
result['kernel_errors'] = subprocess.run(['sudo','-n','journalctl','-k','--since','2026-10-12 22:42:12 UTC','--no-pager','--grep','Out of memory|oom-kill|Killed process|I/O error|EXT4-fs error|under-voltage'],capture_output=True,text=True).stdout
print(json.dumps(result,indent=2))
