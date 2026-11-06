"""Owned, finite ext4/FUSE volume for the existing workload measurement runner."""
import json
import os
from pathlib import Path
import shutil
import subprocess
import time

from validation import file_identity


class QuotaVolume:
    def __init__(self, folder, root):
        self.folder, self.root = folder, root
        self.mount = root / "canonical"
        self.image = folder / "quota.ext4"
        self.daemon = None
        self.daemons = []
        self.capacity = 32 * 1024 * 1024
        self.report = {"image_bytes": self.capacity, "filesystem": "ext4 over FUSE", "commands": []}

    def save(self):
        (self.folder / "quota-filesystem.json").write_text(json.dumps(self.report, indent=2) + "\n")

    def __enter__(self):
        for tool in ("mke2fs", "fuse2fs", "fusermount"):
            path = shutil.which(tool)
            if path is None:
                raise RuntimeError(f"selected disk-quota drill requires {tool}")
            self.report[tool] = {"path": path, **file_identity(Path(path).resolve())}
        self.root.mkdir()
        self.mount.mkdir()
        with self.image.open("xb") as stream:
            stream.truncate(self.capacity)
        command = ["mke2fs", "-q", "-t", "ext4", "-F", "-m", "0", str(self.image)]
        self.report["commands"].append(command)
        with (self.folder / "quota-mkfs.log").open("wb") as log:
            subprocess.run(command, stdout=log, stderr=subprocess.STDOUT, check=True, timeout=30)
        try:
            self.start()
        except BaseException:
            self.close()
            raise
        return self

    def start(self):
        command = ["fuse2fs", str(self.image), str(self.mount), "-f", "-o", "fakeroot"]
        self.report["commands"].append(command)
        self.started = time.monotonic()
        with (self.folder / "quota-fuse.log").open("ab") as log:
            self.daemon = subprocess.Popen(command, stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
        while not os.path.ismount(self.mount):
            if self.daemon.poll() is not None:
                raise RuntimeError(f"FUSE mount failed; see {self.folder / 'quota-fuse.log'}")
            if time.monotonic() - self.started > 10:
                raise TimeoutError("FUSE mount readiness timed out")
            time.sleep(.01)
        stat = os.statvfs(self.mount)
        if not 0 < stat.f_blocks * stat.f_frsize <= self.capacity:
            raise RuntimeError("mounted filesystem does not have the expected finite capacity")
        self.report.setdefault("mounts", []).append({"pid": self.daemon.pid, "filesystem_bytes": stat.f_blocks * stat.f_frsize,
            "available_bytes": stat.f_bavail * stat.f_frsize, "mountinfo": [line for line in Path("/proc/self/mountinfo").read_text().splitlines()
                if line.split()[4] == str(self.mount)]})
        if not self.report["mounts"][-1]["mountinfo"]:
            raise RuntimeError("owned quota mount missing from mountinfo")
        self.save()

    def close(self):
        if os.path.ismount(self.mount):
            command = ["fusermount", "-u", str(self.mount)]
            self.report["commands"].append(command)
            subprocess.run(command, stdout=subprocess.PIPE, stderr=subprocess.PIPE, check=True, timeout=10)
        if self.daemon is not None:
            process = self.daemon
            if process.returncode is None:
                deadline = time.monotonic() + 10
                while True:
                    pid, status, usage = os.wait4(process.pid, os.WNOHANG)
                    if pid:
                        process.returncode = os.waitstatus_to_exitcode(status)
                        self.daemons.append({"pid": pid, "exit_code": process.returncode,
                            "wall_seconds": time.monotonic() - self.started, "user_cpu_seconds": usage.ru_utime,
                            "system_cpu_seconds": usage.ru_stime, "peak_rss_bytes": usage.ru_maxrss * 1024})
                        break
                    if time.monotonic() > deadline:
                        process.kill()
                        process.wait()
                        raise TimeoutError("FUSE daemon failed to stop after unmount")
                    time.sleep(.01)
            self.daemon = None
            self.report["daemons"] = self.daemons
            self.save()
            if process.returncode:
                raise RuntimeError(f"FUSE daemon exited {process.returncode}")

    @property
    def pids(self):
        return [self.daemon.pid] if self.daemon else []

    def release_and_remount(self):
        stat = os.statvfs(self.mount)
        self.report["after_failed_workers"] = {"available_bytes": stat.f_bavail * stat.f_frsize,
            "filler_bytes": (self.mount / "quota-filler").stat().st_size}
        (self.mount / "quota-filler").unlink()
        self.close()
        self.report["after_release_image"] = file_identity(self.image)
        self.start()

    def remove(self):
        self.close()
        self.report["removed_original_image"] = file_identity(self.image)
        self.image.unlink()
        self.mount.rmdir()
        self.report["original_image_present"] = False
        self.save()

    def __exit__(self, *_):
        self.close()
