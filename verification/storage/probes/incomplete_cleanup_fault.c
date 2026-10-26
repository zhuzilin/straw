/* Linux test shim only. One armed operation on one private payload/path
 * returns EIO before or after the real call. It never touches other paths. */
#define _GNU_SOURCE
#include <dlfcn.h>
#include <errno.h>
#include <fcntl.h>
#include <limits.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

static int armed(const char *kind, const char *path) {
    const char *target = getenv("STRAW_INCOMPLETE_TARGET");
    const char *selected = getenv("STRAW_INCOMPLETE_KIND");
    const char *arm = getenv("STRAW_INCOMPLETE_ARM");
    const char *mode = getenv("STRAW_INCOMPLETE_MODE");
    if (!target || !selected || !arm || !mode || strcmp(target, path) ||
        strcmp(selected, kind) || (strcmp(mode, "before") && strcmp(mode, "after"))) return 0;
    int (*real_unlink)(const char *) = dlsym(RTLD_NEXT, "unlink");
    return real_unlink(arm) == 0;
}

static int fd_armed(const char *kind, int fd) {
    char proc[64], path[PATH_MAX];
    snprintf(proc, sizeof(proc), "/proc/self/fd/%d", fd);
    ssize_t size = readlink(proc, path, sizeof(path) - 1);
    if (size < 0) return 0;
    path[size] = 0;
    return armed(kind, path);
}

static int before(void) { return !strcmp(getenv("STRAW_INCOMPLETE_MODE"), "before"); }

static int finish(int actual) {
    const char *log = getenv("STRAW_INCOMPLETE_LOG");
    int out = log ? open(log, O_WRONLY | O_CREAT | O_APPEND, 0600) : -1;
    if (out >= 0) {
        dprintf(out, "%s\t%s\t%d\t%s\n", getenv("STRAW_INCOMPLETE_KIND"),
                getenv("STRAW_INCOMPLETE_MODE"), actual, getenv("STRAW_INCOMPLETE_TARGET"));
        close(out);
    }
    errno = EIO;
    return -1;
}

int fsync(int fd) {
    int (*real_call)(int) = dlsym(RTLD_NEXT, "fsync");
    if (!fd_armed("fsync", fd)) return real_call(fd);
    return finish(before() ? -2 : real_call(fd));
}

int ftruncate(int fd, off_t length) {
    int (*real_call)(int, off_t) = dlsym(RTLD_NEXT, "ftruncate");
    if (!fd_armed("truncate", fd)) return real_call(fd, length);
    return finish(before() ? -2 : real_call(fd, length));
}

int ftruncate64(int fd, off64_t length) {
    int (*real_call)(int, off64_t) = dlsym(RTLD_NEXT, "ftruncate64");
    if (!fd_armed("truncate", fd)) return real_call(fd, length);
    return finish(before() ? -2 : real_call(fd, length));
}

int unlink(const char *path) {
    int (*real_call)(const char *) = dlsym(RTLD_NEXT, "unlink");
    if (!armed("unlink", path)) return real_call(path);
    return finish(before() ? -2 : real_call(path));
}

int unlinkat(int dirfd, const char *path, int flags) {
    int (*real_call)(int, const char *, int) = dlsym(RTLD_NEXT, "unlinkat");
    /* Rust remove_file currently supplies an absolute path. Never intercept
     * relative paths without resolving their dirfd identity. */
    if (*path != '/' || !armed("unlink", path)) return real_call(dirfd, path, flags);
    return finish(before() ? -2 : real_call(dirfd, path, flags));
}
