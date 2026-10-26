/* Linux experiment shim. One armed private payload open/seek fails before or
 * after its real effect. An opened descriptor is closed before returning EIO.
 * This is fault injection, not a claim that the deployment produced the fault. */
#define _GNU_SOURCE
#include <dlfcn.h>
#include <errno.h>
#include <fcntl.h>
#include <limits.h>
#include <stdarg.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

static int armed(const char *kind, const char *path) {
    const char *root = getenv("STRAW_PREP_ROOT"), *selected = getenv("STRAW_PREP_KIND");
    const char *mode = getenv("STRAW_PREP_MODE"), *arm = getenv("STRAW_PREP_ARM");
    if (!root || !selected || !mode || !arm || strcmp(kind, selected) ||
        (strcmp(mode, "before") && strcmp(mode, "after"))) return 0;
    size_t n = strlen(path), r = strlen(root);
    if (n <= r || strncmp(root, path, r) || path[r] != '/' ||
        !((n >= 8 && !strcmp(path + n - 8, ".partial")) ||
          (n >= 5 && !strcmp(path + n - 5, ".pack")))) return 0;
    int (*remove_arm)(const char *) = dlsym(RTLD_NEXT, "unlink");
    return remove_arm(arm) == 0;
}

static int before(void) { return !strcmp(getenv("STRAW_PREP_MODE"), "before"); }

static void trace(const char *kind, const char *path, long long actual) {
    const char *log = getenv("STRAW_PREP_LOG");
    int (*real_open)(const char *, int, ...) = dlsym(RTLD_NEXT, "open");
    int fd = log ? real_open(log, O_WRONLY | O_CREAT | O_APPEND, 0600) : -1;
    if (fd >= 0) {
        dprintf(fd, "%s\t%s\t%lld\t%s\n", kind, getenv("STRAW_PREP_MODE"), actual, path);
        close(fd);
    }
    errno = EIO;
}

static int open_call(const char *name, const char *path, int flags, mode_t mode) {
    int (*real_call)(const char *, int, ...) = dlsym(RTLD_NEXT, name);
    if (!(flags & O_CREAT) || !armed("open", path)) return real_call(path, flags, mode);
    int actual = before() ? -2 : real_call(path, flags, mode);
    if (actual >= 0) close(actual);
    trace("open", path, actual);
    return -1;
}

#define OPEN_WRAPPER(name) \
int name(const char *path, int flags, ...) { \
    mode_t mode = 0; \
    if ((flags & O_CREAT) || (flags & O_TMPFILE) == O_TMPFILE) { \
        va_list args; va_start(args, flags); mode = va_arg(args, mode_t); va_end(args); \
    } \
    return open_call(#name, path, flags, mode); \
}
OPEN_WRAPPER(open)
OPEN_WRAPPER(open64)

static int seek_armed(int fd, int whence, char *path) {
    if (whence != SEEK_END) return 0;
    char proc[64];
    snprintf(proc, sizeof(proc), "/proc/self/fd/%d", fd);
    ssize_t n = readlink(proc, path, PATH_MAX - 1);
    if (n < 0) return 0;
    path[n] = 0;
    return armed("seek", path);
}

#define SEEK_WRAPPER(name, type) \
type name(int fd, type offset, int whence) { \
    type (*real_call)(int, type, int) = dlsym(RTLD_NEXT, #name); \
    char path[PATH_MAX]; \
    if (!seek_armed(fd, whence, path)) return real_call(fd, offset, whence); \
    type actual = before() ? -2 : real_call(fd, offset, whence); \
    trace("seek", path, (long long)actual); \
    return -1; \
}
SEEK_WRAPPER(lseek, off_t)
SEEK_WRAPPER(lseek64, off64_t)
