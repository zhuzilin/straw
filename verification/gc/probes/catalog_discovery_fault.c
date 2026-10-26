/* Linux test shim. One armed stat ENOENT/EIO, or one read-only open EIO,
 * on the supervisor's exact private catalog path. Never linked into Straw. */
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

static int inject(const char *path, int opening) {
    const char *target = getenv("STRAW_DISCOVERY_PATH");
    const char *arm = getenv("STRAW_DISCOVERY_ARM");
    const char *kind = getenv("STRAW_DISCOVERY_FAULT");
    if (!target || !arm || !kind || !path || strcmp(path, target)) return 0;
    if (opening ? strcmp(kind, "open-error") :
        (strcmp(kind, "stat-error") && strcmp(kind, "stat-missing"))) return 0;
    if (unlink(arm)) return 0;
    char marker[PATH_MAX];
    int n = snprintf(marker, sizeof(marker), "%s.fired", arm);
    if (n > 0 && n < (int)sizeof(marker)) {
        int fd = open(marker, O_WRONLY | O_CREAT | O_EXCL, 0600);
        if (fd >= 0) close(fd);
    }
    errno = !strcmp(kind, "stat-missing") ? ENOENT : EIO;
    return 1;
}
int statx(int dirfd, const char *path, int flags, unsigned int mask, struct statx *buf) {
    static int (*real)(int, const char *, int, unsigned int, struct statx *);
    if (!real) real = dlsym(RTLD_NEXT, "statx");
    if (inject(path, 0)) return -1;
    return real(dirfd, path, flags, mask, buf);
}
int stat64(const char *path, struct stat64 *buf) {
    static int (*real)(const char *, struct stat64 *);
    if (!real) real = dlsym(RTLD_NEXT, "stat64");
    if (inject(path, 0)) return -1;
    return real(path, buf);
}
int fstatat64(int dirfd, const char *path, struct stat64 *buf, int flags) {
    static int (*real)(int, const char *, struct stat64 *, int);
    if (!real) real = dlsym(RTLD_NEXT, "fstatat64");
    if (inject(path, 0)) return -1;
    return real(dirfd, path, buf, flags);
}
#define OPEN_WRAPPER(name) \
int name(const char *path, int flags, ...) { \
    static int (*real)(const char *, int, ...); \
    if (!real) real = dlsym(RTLD_NEXT, #name); \
    mode_t mode = 0; \
    if ((flags & O_CREAT) || (flags & O_TMPFILE) == O_TMPFILE) { \
        va_list args; va_start(args, flags); mode = va_arg(args, mode_t); va_end(args); \
    } \
    if ((flags & O_ACCMODE) == O_RDONLY && inject(path, 1)) return -1; \
    return real(path, flags, mode); \
}
OPEN_WRAPPER(open)
OPEN_WRAPPER(open64)
