/* Linux deployment-test shim, never linked into Straw. Fail exactly one armed
 * fsync on the private catalog path selected by catalog_replay_probe.py. */
#define _GNU_SOURCE
#include <dlfcn.h>
#include <unistd.h>
#include <errno.h>
#include <stdlib.h>
#include <string.h>
#include <stdio.h>
#include <limits.h>
int fsync(int fd) {
    static int (*real_fsync)(int);
    if (!real_fsync) real_fsync = dlsym(RTLD_NEXT, "fsync");
    const char *target = getenv("STRAW_FAIL_SYNC_PATH");
    const char *armed = getenv("STRAW_FAIL_SYNC_ARM");
    if (target && armed) {
        char proc[64], path[PATH_MAX];
        snprintf(proc, sizeof(proc), "/proc/self/fd/%d", fd);
        ssize_t n = readlink(proc, path, sizeof(path)-1);
        if (n >= 0) {
            path[n] = 0;
            if (!strcmp(path, target) && !unlink(armed)) {
                errno = EIO;
                return -1;
            }
        }
    }
    return real_fsync(fd);
}
