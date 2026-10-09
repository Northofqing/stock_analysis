"""Small private filesystem/calendar helpers for local reliability tools (Python 3.9)."""
import datetime as dt
import hashlib
import json
import os
from pathlib import Path
import re
import stat
import uuid

SHANGHAI = dt.timezone(dt.timedelta(hours=8))
MAX_FILE = 8 * 1024 * 1024


def absolute(value):
    path = Path(value)
    if not path.is_absolute() or '..' in path.parts:
        raise ValueError('absolute path without .. required')
    return path


def digest(raw):
    return hashlib.sha256(raw).hexdigest()


def clock(value=None):
    now = dt.datetime.fromisoformat(value.replace('Z', '+00:00')) if value else dt.datetime.now(SHANGHAI)
    if now.tzinfo is None:
        raise ValueError('observed-at requires a timezone')
    return now.astimezone(SHANGHAI)


def date(value):
    if not isinstance(value, str) or not re.fullmatch(r'\d{4}-\d{2}-\d{2}', value):
        raise ValueError('invalid date')
    return dt.date.fromisoformat(value)


def identity(info):
    return [info.st_dev, info.st_ino, info.st_size, info.st_mtime_ns, info.st_ctime_ns]


def open_dir(path, create=False, private=False):
    """Walk every component through directory descriptors; never follow symlinks."""
    path = absolute(path)
    fd = os.open('/', os.O_RDONLY | os.O_DIRECTORY)
    try:
        for part in path.parts[1:]:
            if create:
                try:
                    os.mkdir(part, 0o700, dir_fd=fd)
                    os.fsync(fd)  # New output roots must survive before any source prune.
                except FileExistsError:
                    pass
            child = os.open(part, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=fd)
            os.close(fd)
            fd = child
        info = os.fstat(fd)
        if private and (info.st_uid != os.getuid() or stat.S_IMODE(info.st_mode) != 0o700):
            raise ValueError('output directory must be owned by user and mode 0700')
        return fd
    except BaseException:
        os.close(fd)
        raise


def regular(info, private=False):
    if not stat.S_ISREG(info.st_mode) or info.st_nlink != 1 or info.st_mode & 0o022:
        raise ValueError('unsafe regular file')
    if private and (info.st_uid != os.getuid() or stat.S_IMODE(info.st_mode) != 0o600):
        raise ValueError('private file must be owned by user and mode 0600')


def read_at(fd, name, limit=MAX_FILE, private=False):
    child = os.open(name, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK, dir_fd=fd)
    try:
        before = os.fstat(child)
        regular(before, private)
        if before.st_size > limit:
            raise ValueError('file exceeds read limit')
        pieces, size = [], 0
        while True:
            raw = os.read(child, min(65536, limit + 1 - size))
            if not raw:
                break
            pieces.append(raw)
            size += len(raw)
            if size > limit:
                raise ValueError('file exceeds read limit')
        after = os.fstat(child)
        named = os.stat(name, dir_fd=fd, follow_symlinks=False)
        if identity(before) != identity(after) or identity(after) != identity(named):
            raise ValueError('file changed during read')
        return b''.join(pieces), identity(after)
    finally:
        os.close(child)


def read_path(path, limit=MAX_FILE):
    path = absolute(path)
    fd = open_dir(path.parent)
    try:
        return read_at(fd, path.name, limit)[0]
    finally:
        os.close(fd)


def json_bytes(value):
    return (json.dumps(value, ensure_ascii=False, sort_keys=True, indent=2) + '\n').encode()


def write_at(fd, name, raw, replace=False):
    """Durable exclusive publication, or atomic replacement of private state."""
    if '/' in name or name in ('.', '..'):
        raise ValueError('leaf name required')
    if replace:
        try:
            regular(os.stat(name, dir_fd=fd, follow_symlinks=False), private=True)
        except FileNotFoundError:
            pass
    temp = '.tmp-' + uuid.uuid4().hex
    child = os.open(temp, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600, dir_fd=fd)
    try:
        with os.fdopen(child, 'wb') as stream:
            stream.write(raw)
            stream.flush()
            os.fsync(stream.fileno())
        if replace:
            os.replace(temp, name, src_dir_fd=fd, dst_dir_fd=fd)
        else:
            os.link(temp, name, src_dir_fd=fd, dst_dir_fd=fd, follow_symlinks=False)
            os.unlink(temp, dir_fd=fd)
        os.fsync(fd)
    finally:
        try:
            os.unlink(temp, dir_fd=fd)
        except FileNotFoundError:
            pass


def exclusive_json(path, value):
    path = absolute(path)
    fd = open_dir(path.parent, private=True)
    try:
        write_at(fd, path.name, json_bytes(value))
    finally:
        os.close(fd)


class Calendar:
    def __init__(self, path, extra=()):
        raw = read_path(path, 128 * 1024)
        self.sha256 = digest(raw)
        text = raw.decode('utf-8')
        declarations = re.findall(r'^# year=([0-9,]+)$', text, re.M)
        if len(declarations) != 1:
            raise ValueError('calendar coverage declaration missing')
        self.years = set(int(y) for y in declarations[0].split(','))
        self.holidays = set(date(line.strip()) for line in text.splitlines()
                            if line.strip() and not line.lstrip().startswith('#'))
        if not self.years or any(d.year not in self.years for d in self.holidays):
            raise ValueError('calendar invalid coverage')
        self.extra = sorted(date(x).isoformat() for x in extra)
        self.holidays.update(date(x) for x in self.extra)
        self.extra_sha256 = digest(json_bytes(self.extra))

    def trading(self, day):
        if day.year not in self.years:
            raise ValueError('calendar_year_unavailable')
        return day.weekday() < 5 and day not in self.holidays

    def session(self, now):
        now = now.astimezone(SHANGHAI)
        if not self.trading(now.date()):
            return 'Closed'
        t = now.time().replace(tzinfo=None)
        if dt.time(9, 15) <= t < dt.time(9, 25):
            return 'Auction'
        if dt.time(9, 30) <= t < dt.time(11, 30):
            return 'Morning'
        if dt.time(11, 30) <= t < dt.time(13):
            return 'LunchBreak'
        if dt.time(13) <= t < dt.time(15):
            return 'Afternoon'
        return 'AfterHours' if t >= dt.time(15) else 'Closed'
