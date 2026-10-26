/* Linux measurement shim; counts only readlink calls under the private pool.
 * It neither changes responses nor asserts metadata-service RPC counts. */
#define _GNU_SOURCE
#include <dlfcn.h>
#include <stdatomic.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
static _Atomic unsigned long calls;
static int selected(const char *path) {
    const char *root = getenv("STRAW_LOOKUP_ROOT");
    return root && path && !strncmp(path, root, strlen(root)) &&
        (path[strlen(root)] == '/' || path[strlen(root)] == '\0');
}
ssize_t readlink(const char *path, char *buf, size_t size) {
    static ssize_t (*real)(const char *, char *, size_t);
    if (!real) real = dlsym(RTLD_NEXT, "readlink");
    if (selected(path)) atomic_fetch_add(&calls, 1);
    return real(path, buf, size);
}
ssize_t readlinkat(int fd, const char *path, char *buf, size_t size) {
    static ssize_t (*real)(int, const char *, char *, size_t);
    if (!real) real = dlsym(RTLD_NEXT, "readlinkat");
    if (selected(path)) atomic_fetch_add(&calls, 1);
    return real(fd, path, buf, size);
}
/* Called explicitly by the probe so setup/cleanup are outside the count. */
unsigned long straw_lookup_count(void) { return atomic_load(&calls); }
void straw_lookup_reset(void) { atomic_store(&calls, 0); }
