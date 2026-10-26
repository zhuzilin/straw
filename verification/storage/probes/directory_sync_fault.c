/* Linux deployment-test shim, never linked into Straw. Only the selected
 * private directory can fail; optional logs record real directory barriers.
 * Without a target/log, healthy timing calls go straight to real fsync. */
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

int fsync(int fd) {
    static int (*real_fsync)(int);
    if (!real_fsync) real_fsync = dlsym(RTLD_NEXT, "fsync");
    const char *target = getenv("STRAW_DIRECTORY_FAIL");
    const char *arm = getenv("STRAW_DIRECTORY_ARM");
    const char *log = getenv("STRAW_DIRECTORY_LOG");
    const char *root = getenv("STRAW_DIRECTORY_ROOT");
    if (!target && !log) return real_fsync(fd);
    char proc[64], path[PATH_MAX];
    snprintf(proc, sizeof(proc), "/proc/self/fd/%d", fd);
    ssize_t n = readlink(proc, path, sizeof(path)-1);
    if (n < 0) return real_fsync(fd);
    path[n] = 0;
    int injected = target && arm && !strcmp(target, path) && !unlink(arm);
    int result = injected ? -1 : real_fsync(fd);
    int saved = injected ? EIO : errno;
    struct stat info;
    if (root && log && (!strcmp(root, "/") || (!strncmp(path, root, strlen(root)) &&
        (path[strlen(root)] == '/' || path[strlen(root)] == 0))) &&
        !fstat(fd, &info) && S_ISDIR(info.st_mode)) {
        int out = open(log, O_WRONLY | O_CREAT | O_APPEND, 0600);
        if (out >= 0) {
            dprintf(out, "%d\t%d\t%s\n", result, injected, path);
            close(out);
        }
    }
    errno = saved;
    return result;
}
