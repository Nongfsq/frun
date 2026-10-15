# 仓库里的脚本要临时目录或无头 Edge，都从这里拿，不自己建。
#
#   sys.path.insert(0, str(ROOT / 'tools')); from headless_edge import Edge, Scratch
#   with Scratch() as d: ...                          # 临时目录 d（pathlib.Path）
#   with Edge() as edge:
#       edge.run('--screenshot=out.png', url)          # 跑完就退出的：截图
#       proc = edge.popen('--remote-debugging-port=9222', url)   # 常驻的：调试接口
#
# 为什么：2026-10-14/15 各脚本自己在 C: 盘的 %TEMP% 建配置目录，Edge 被打断或脱离父进程后目录留下，
# 攒了 130 GB；Edge 自己又往 %TEMP% 解了 2.6 GB 扩展包。所以这里：
# - 所有东西放在仓库的 .cache/run/ 下（和代码同一块盘，已被 git 忽略），Edge 的 TEMP/TMP 也指过来；
# - Edge 一律不登录、不同步、不装扩展、不做首次运行导入；
# - Edge 的整棵进程树装进一个 Job，本脚本退出（哪怕崩溃、被强杀）时系统会把它们一起结束；
# - 每次开始先清掉没人在用的旧目录（比如断电留下的），清完还剩太多就报错停下。
# tests/script_hygiene.rs 会拦下绕开这里、自己建临时目录或启动浏览器的写法。
import ctypes, os, pathlib, shutil, subprocess, time, uuid
from ctypes import wintypes

ROOT = pathlib.Path(__file__).resolve().parents[1]
RUNS = ROOT / '.cache' / 'run'
EDGE = next((p for p in (r'C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe',
                         r'C:\Program Files\Microsoft\Edge\Application\msedge.exe') if os.path.exists(p)), 'msedge.exe')
QUIET = ['--headless=new', '--no-first-run', '--no-default-browser-check', '--disable-sync', '--disable-extensions',
         '--disable-component-update', '--disable-features=msImplicitSignin,msEdgeSyncWithOtherBrowsers']
# 清扫之后仍有这么多目录在用，或者总大小超过这个数，多半是哪里在漏：停下来查
MAX_LIVE, MAX_BYTES = 64, 2 << 30

k32 = ctypes.WinDLL('kernel32', use_last_error=True)
k32.CreateJobObjectW.restype = wintypes.HANDLE
k32.CreateJobObjectW.argtypes = [ctypes.c_void_p, wintypes.LPCWSTR]
k32.SetInformationJobObject.argtypes = [wintypes.HANDLE, ctypes.c_int, ctypes.c_void_p, wintypes.DWORD]
k32.AssignProcessToJobObject.argtypes = [wintypes.HANDLE, wintypes.HANDLE]
k32.QueryInformationJobObject.argtypes = [wintypes.HANDLE, ctypes.c_int, ctypes.c_void_p, wintypes.DWORD, ctypes.c_void_p]
k32.TerminateJobObject.argtypes = [wintypes.HANDLE, wintypes.UINT]
k32.CloseHandle.argtypes = [wintypes.HANDLE]


class _Limits(ctypes.Structure):
    # JOBOBJECT_EXTENDED_LIMIT_INFORMATION；只用到 LimitFlags，其余占位
    _fields_ = [('PerProcessUserTimeLimit', ctypes.c_int64), ('PerJobUserTimeLimit', ctypes.c_int64),
                ('LimitFlags', wintypes.DWORD), ('MinimumWorkingSetSize', ctypes.c_size_t),
                ('MaximumWorkingSetSize', ctypes.c_size_t), ('ActiveProcessLimit', wintypes.DWORD),
                ('Affinity', ctypes.c_size_t), ('PriorityClass', wintypes.DWORD), ('SchedulingClass', wintypes.DWORD),
                ('IoCounters', ctypes.c_uint64 * 6), ('ProcessMemoryLimit', ctypes.c_size_t),
                ('JobMemoryLimit', ctypes.c_size_t), ('PeakProcessMemoryUsed', ctypes.c_size_t),
                ('PeakJobMemoryUsed', ctypes.c_size_t)]


class _Accounting(ctypes.Structure):
    # JOBOBJECT_BASIC_ACCOUNTING_INFORMATION
    _fields_ = [('TotalUserTime', ctypes.c_int64), ('TotalKernelTime', ctypes.c_int64),
                ('ThisPeriodTotalUserTime', ctypes.c_int64), ('ThisPeriodTotalKernelTime', ctypes.c_int64),
                ('TotalPageFaultCount', wintypes.DWORD), ('TotalProcesses', wintypes.DWORD),
                ('ActiveProcesses', wintypes.DWORD), ('TotalTerminatedProcesses', wintypes.DWORD)]


def _size(path):
    return sum(f.stat().st_size for f in pathlib.Path(path).rglob('*') if f.is_file())


def _remove(path, tries=40):
    for _ in range(tries):
        shutil.rmtree(path, ignore_errors=True)
        if not os.path.exists(path):
            return True
        time.sleep(0.25)
    return False


def sweep():
    """删掉没人在用的旧目录。目录有主时，主人一直开着里面的 owner 文件，改名会失败；改名成功就说明没人用了。"""
    RUNS.mkdir(parents=True, exist_ok=True)
    for d in RUNS.iterdir():
        if d.name.endswith('.trash'):
            _remove(d, 4)
            continue
        trash = d.with_name(d.name + '.trash')
        try:
            d.rename(trash)
        except OSError:
            continue  # 有人在用
        _remove(trash, 4)
    live = [d for d in RUNS.iterdir() if not d.name.endswith('.trash')]
    if len(live) > MAX_LIVE or sum(_size(d) for d in live) > MAX_BYTES:
        raise RuntimeError(f'{RUNS} 里有 {len(live)} 个目录还在用，超出上限；先查是哪个脚本没收尾：\n  '
                           + '\n  '.join(d.name for d in live[:20]))


class Scratch:
    """仓库 .cache/run/ 下的一个临时目录；with 结束时删掉。"""

    def __enter__(self):
        sweep()
        self.dir = RUNS / f'{os.getpid()}-{uuid.uuid4().hex[:8]}'
        self.dir.mkdir(parents=True)
        self._owner = open(self.dir / 'owner', 'w')
        return self.dir

    def __exit__(self, *exc):
        self._owner.close()
        if not _remove(self.dir):
            print(f'could not remove {self.dir}', flush=True)


class Edge(Scratch):
    """一个无头 Edge 的配置目录和临时目录；它启动的进程都在一个 Job 里，with 结束时一起结束。"""

    def __enter__(self):
        super().__enter__()
        self.profile, self.tmp = self.dir / 'profile', self.dir / 'tmp'
        self.tmp.mkdir()
        self.env = {**os.environ, 'TEMP': str(self.tmp), 'TMP': str(self.tmp)}
        self.job = k32.CreateJobObjectW(None, None)
        limits = _Limits(LimitFlags=0x2000)  # JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE
        if not self.job or not k32.SetInformationJobObject(self.job, 9, ctypes.byref(limits), ctypes.sizeof(limits)):
            super().__exit__()
            raise ctypes.WinError(ctypes.get_last_error())
        return self

    def popen(self, *args):
        p = subprocess.Popen([EDGE, *QUIET, f'--user-data-dir={self.profile}', *args], env=self.env,
                             stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        if not k32.AssignProcessToJobObject(self.job, int(p._handle)):
            p.kill()
            raise ctypes.WinError(ctypes.get_last_error())
        return p

    def alive(self):
        info = _Accounting()
        k32.QueryInformationJobObject(self.job, 1, ctypes.byref(info), ctypes.sizeof(info), None)
        return info.ActiveProcesses

    def run(self, *args, timeout=60):
        """启动、等它和它开的所有进程都退出（下一次 run 才能用同一个配置目录）。"""
        p = self.popen(*args)
        end = time.time() + timeout
        while self.alive():
            if time.time() > end:
                k32.TerminateJobObject(self.job, 1)
                raise TimeoutError(f'edge {args} did not exit in {timeout} s')
            time.sleep(0.05)
        return p.wait()

    def __exit__(self, *exc):
        k32.TerminateJobObject(self.job, 0)
        k32.CloseHandle(self.job)
        # Job 刚建好、Edge 还没装进去就分出的子进程会漏掉：按配置目录再找一遍
        if self.alive_elsewhere():
            subprocess.run(['powershell', '-NoProfile', '-Command',
                            f"Get-CimInstance Win32_Process -Filter \"Name='msedge.exe'\" | Where-Object {{ $_.CommandLine -like '*{self.dir.name}*' }}"
                            " | ForEach-Object { Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue }"],
                           stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        super().__exit__()

    def alive_elsewhere(self):
        """配置目录还被占着，说明有进程没跟着 Job 结束。"""
        for _ in range(20):
            try:
                (self.profile / 'lockfile').unlink(missing_ok=True)
                return False
            except OSError:
                time.sleep(0.1)
        return True
