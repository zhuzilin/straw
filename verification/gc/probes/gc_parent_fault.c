/* Linux experiment shim. Only an exact private directory may be faulted.
 * Local /tmp arm/trace files are outside the tested storage pool. */
#define _GNU_SOURCE
#include <dlfcn.h>
#include <errno.h>
#include <fcntl.h>
#include <limits.h>
#include <stdarg.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <unistd.h>

static int selected(const char *path) {
    const char *target = getenv("STRAW_PARENT_TARGET");
    return target && path && !strcmp(target, path);
}
static int consume(const char *name) {
    const char *arm = getenv(name);
    return arm && !unlink(arm);
}
static void trace(const char *kind, int actual) {
    const char *log = getenv("STRAW_PARENT_LOG");
    int saved = errno;
    int fd = log ? open(log, O_WRONLY | O_CREAT | O_APPEND, 0600) : -1;
    if (fd >= 0) {
        dprintf(fd, "%s\t%d\n", kind, actual);
        close(fd);
    }
    errno = saved;
}
static int open_error(const char *path) {
    const char *mode = getenv("STRAW_PARENT_MODE");
    if (!selected(path) || !mode || (!strstr(mode, "open_") && !strstr(mode, "sync_"))) return 0;
    if (!consume("STRAW_PARENT_OPEN_ARM")) return 0;
    trace("open", -2);
    errno = !strcmp(mode, "open_error") ? EIO : ENOENT;
    return 1;
}
#define OPEN_WRAPPER(name) \
int name(const char *path, int flags, ...) { \
    int (*real_call)(const char *, int, ...) = dlsym(RTLD_NEXT, #name); \
    mode_t mode = 0; \
    if ((flags & O_CREAT) || (flags & O_TMPFILE) == O_TMPFILE) { \
        va_list args; va_start(args, flags); mode = va_arg(args, mode_t); va_end(args); \
    } \
    if (!(flags & O_CREAT) && (flags & O_ACCMODE) == O_RDONLY && open_error(path)) return -1; \
    return real_call(path, flags, mode); \
}
OPEN_WRAPPER(open)
OPEN_WRAPPER(open64)

int fsync(int fd) {
    int (*real_call)(int) = dlsym(RTLD_NEXT, "fsync");
    const char *mode = getenv("STRAW_PARENT_MODE");
    char proc[64], path[PATH_MAX];
    snprintf(proc, sizeof(proc), "/proc/self/fd/%d", fd);
    ssize_t n = readlink(proc, path, sizeof(path) - 1);
    if (n < 0) return real_call(fd);
    path[n] = 0;
    if (!selected(path) || !mode || !strstr(mode, "sync_") || !consume("STRAW_PARENT_EFFECT_ARM")) return real_call(fd);
    int actual = !strcmp(mode, "sync_before") ? -2 : real_call(fd);
    trace("fsync", actual);
    errno = EIO;
    return -1;
}

int mkdir(const char *path, mode_t permission) {
    int (*real_call)(const char *, mode_t) = dlsym(RTLD_NEXT, "mkdir");
    const char *mode = getenv("STRAW_PARENT_MODE");
    if (!selected(path) || !mode || !strstr(mode, "mkdir_")) return real_call(path, permission);
    if (!strcmp(mode, "mkdir_before") && consume("STRAW_PARENT_EFFECT_ARM")) {
        trace("mkdir", -2); errno = EIO; return -1;
    }
    int actual = real_call(path, permission);
    if (!actual && !strcmp(mode, "mkdir_after") && consume("STRAW_PARENT_EFFECT_ARM")) {
        trace("mkdir", actual); errno = EIO; return -1;
    }
    return actual;
}
