/* Linux deployment-test shim, never linked into Straw. Only payload files
 * under the selected private pool are observed. One armed fsync fails either
 * before its real call or after a successful real call; traces stay in /tmp. */
#define _GNU_SOURCE
#include <dlfcn.h>
#include <errno.h>
#include <fcntl.h>
#include <limits.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

int fsync(int fd) {
    static int (*real_fsync)(int);
    if (!real_fsync) real_fsync = dlsym(RTLD_NEXT, "fsync");
    const char *root = getenv("STRAW_PENDING_ROOT");
    const char *mode = getenv("STRAW_PENDING_MODE");
    const char *arm = getenv("STRAW_PENDING_ARM");
    const char *log = getenv("STRAW_PENDING_LOG");
    if (!root || !log) return real_fsync(fd);
    char proc[64], path[PATH_MAX];
    snprintf(proc, sizeof(proc), "/proc/self/fd/%d", fd);
    ssize_t n = readlink(proc, path, sizeof(path)-1);
    if (n < 0) return real_fsync(fd);
    path[n] = 0;
    size_t len = strlen(path), prefix = strlen(root);
    if (strncmp(path, root, prefix) || path[prefix] != '/' ||
        !((len >= 8 && !strcmp(path+len-8, ".partial")) ||
          (len >= 7 && !strcmp(path+len-7, ".sealed")))) return real_fsync(fd);
    int inject = mode && arm && (!strcmp(mode, "before") || !strcmp(mode, "after")) && !unlink(arm);
    int perform = !(inject && !strcmp(mode, "before"));
    int actual = perform ? real_fsync(fd) : -2;
    int result = inject ? -1 : actual;
    int saved = inject ? EIO : errno;
    int out = open(log, O_WRONLY | O_CREAT | O_APPEND, 0600);
    if (out >= 0) {
        dprintf(out, "%d\t%d\t%d\t%d\t%s\n", result, inject, perform, actual, path);
        close(out);
    }
    errno = saved;
    return result;
}
