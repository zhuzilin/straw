/* Linux deployment-test shim, never linked into Straw. Return one stale
 * ENOENT for metadata on the exact private pack named by the probe. */
#define _GNU_SOURCE
#include <dlfcn.h>
#include <errno.h>
#include <fcntl.h>
#include <limits.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <unistd.h>

static int stale(const char *path) {
    const char *target = getenv("STRAW_STALE_STAT_PATH");
    const char *arm = getenv("STRAW_STALE_STAT_ARM");
    if (target && arm && path && !strcmp(path, target) && !unlink(arm)) {
        char marker[PATH_MAX];
        int n = snprintf(marker, sizeof(marker), "%s.fired", arm);
        if (n > 0 && n < (int)sizeof(marker)) {
            int fd = open(marker, O_WRONLY | O_CREAT | O_EXCL, 0600);
            if (fd >= 0) close(fd);
        }
        errno = ENOENT;
        return 1;
    }
    return 0;
}
int statx(int dirfd, const char *path, int flags, unsigned int mask, struct statx *buf) {
    static int (*real)(int, const char *, int, unsigned int, struct statx *);
    if (!real) real = dlsym(RTLD_NEXT, "statx");
    if (stale(path)) return -1;
    return real(dirfd, path, flags, mask, buf);
}
int stat64(const char *path, struct stat64 *buf) {
    static int (*real)(const char *, struct stat64 *);
    if (!real) real = dlsym(RTLD_NEXT, "stat64");
    if (stale(path)) return -1;
    return real(path, buf);
}
int fstatat64(int dirfd, const char *path, struct stat64 *buf, int flags) {
    static int (*real)(int, const char *, struct stat64 *, int);
    if (!real) real = dlsym(RTLD_NEXT, "fstatat64");
    if (stale(path)) return -1;
    return real(dirfd, path, buf, flags);
}
