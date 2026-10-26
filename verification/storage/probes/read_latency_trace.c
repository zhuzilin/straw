/* Linux diagnostic interposer; no injected errors, sleeps or payload changes.
 * Only absolute paths under STRAW_READ_PROFILE_ROOT and descriptors opened
 * there are selected. This is libc-call timing, not backend RPC accounting.
 * The probe makes its storage calls on one thread; events are thread-local.
 */
#define _GNU_SOURCE
#include <dlfcn.h>
#include <errno.h>
#include <fcntl.h>
#include <stdarg.h>
#include <stdatomic.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <time.h>
#include <unistd.h>

#define MAX_FD 65536
#define MAX_EVENTS 256
static _Atomic unsigned char tracked[MAX_FD];
static _Thread_local int active, count, overflow;
struct event {
    const char *op;
    uint64_t start, wall, cpu, requested;
    long result;
    int error;
};
static _Thread_local struct event events[MAX_EVENTS];
/* Each event's bounded string/numeric representation is under 256 bytes. */
static _Thread_local char output[MAX_EVENTS * 256 + 128];

static uint64_t ns(clockid_t clock) {
    struct timespec t;
    clock_gettime(clock, &t);
    return (uint64_t)t.tv_sec * 1000000000 + t.tv_nsec;
}
static int path_selected(const char *path) {
    const char *root = getenv("STRAW_READ_PROFILE_ROOT");
    if (!root || !root[0] || !path) return 0;
    size_t n = strlen(root);
    return !strncmp(path, root, n) && (path[n] == '/' || !path[n]);
}
static int fd_selected(int fd) {
    return fd >= 0 && fd < MAX_FD && atomic_load(&tracked[fd]);
}
static void track(int fd, int selected) {
    if (fd >= 0 && fd < MAX_FD) atomic_store(&tracked[fd], selected != 0);
    else if (fd >= MAX_FD && selected && active) overflow = 1;
}
static void save(const char *op, uint64_t start, uint64_t cpu,
                 uint64_t requested, long result, int error) {
    uint64_t end = ns(CLOCK_MONOTONIC), used = ns(CLOCK_THREAD_CPUTIME_ID);
    if (count < MAX_EVENTS)
        events[count++] = (struct event){op, start, end-start, used-cpu, requested, result, error};
    else overflow = 1;
    errno = error;
}
#define START uint64_t begin = ns(CLOCK_MONOTONIC), cpu = ns(CLOCK_THREAD_CPUTIME_ID)
#define FINISH(op, requested, result) do { \
    int saved_errno = errno; save(op, begin, cpu, requested, (long)(result), saved_errno); \
} while (0)

void straw_read_profile_begin(void) { count = overflow = 0; active = 1; }
const char *straw_read_profile_end(void) {
    int saved_errno = errno;
    active = 0;
    size_t n = (size_t)snprintf(output, sizeof(output), "{\"overflow\":%d,\"events\":[", overflow);
    for (int i = 0; i < count; i++) {
        struct event *e = &events[i];
        n += (size_t)snprintf(output+n, sizeof(output)-n,
            "%s{\"op\":\"%s\",\"start_ns\":%llu,\"wall_ns\":%llu,\"cpu_ns\":%llu,"
            "\"requested\":%llu,\"result\":%ld,\"errno\":%d}",
            i ? "," : "", e->op, (unsigned long long)e->start,
            (unsigned long long)e->wall, (unsigned long long)e->cpu,
            (unsigned long long)e->requested, e->result, e->error);
    }
    snprintf(output+n, sizeof(output)-n, "]}");
    errno = saved_errno;
    return output;
}

#define OPEN_WRAPPER(name) \
int name(const char *path, int flags, ...) { \
    static _Thread_local int (*real)(const char *, int, ...); \
    if (!real) real = dlsym(RTLD_NEXT, #name); \
    mode_t mode = 0; \
    if ((flags & O_CREAT) || ((flags & O_TMPFILE) == O_TMPFILE)) { \
        va_list a; va_start(a, flags); mode = (mode_t)va_arg(a, int); va_end(a); \
    } \
    int selected = path_selected(path); \
    if (!active || !selected) { int fd = real(path, flags, mode); track(fd, selected); return fd; } \
    START; int fd = real(path, flags, mode); \
    FINISH(#name, 0, fd); track(fd, selected); return fd; \
}
OPEN_WRAPPER(open)
OPEN_WRAPPER(open64)

#define OPENAT_WRAPPER(name) \
int name(int dirfd, const char *path, int flags, ...) { \
    static _Thread_local int (*real)(int, const char *, int, ...); \
    if (!real) real = dlsym(RTLD_NEXT, #name); \
    mode_t mode = 0; \
    if ((flags & O_CREAT) || ((flags & O_TMPFILE) == O_TMPFILE)) { \
        va_list a; va_start(a, flags); mode = (mode_t)va_arg(a, int); va_end(a); \
    } \
    int selected = path_selected(path); \
    if (!active || !selected) { int fd = real(dirfd, path, flags, mode); track(fd, selected); return fd; } \
    START; int fd = real(dirfd, path, flags, mode); \
    FINISH(#name, 0, fd); track(fd, selected); return fd; \
}
OPENAT_WRAPPER(openat)
OPENAT_WRAPPER(openat64)

ssize_t read(int fd, void *buf, size_t n) {
    static _Thread_local ssize_t (*real)(int, void *, size_t);
    if (!real) real = dlsym(RTLD_NEXT, "read");
    if (!active || !fd_selected(fd)) return real(fd, buf, n);
    START; ssize_t r = real(fd, buf, n); FINISH("read", n, r); return r;
}
#define SEEK_WRAPPER(name, type) \
type name(int fd, type offset, int whence) { \
    static _Thread_local type (*real)(int, type, int); \
    if (!real) real = dlsym(RTLD_NEXT, #name); \
    if (!active || !fd_selected(fd)) return real(fd, offset, whence); \
    START; type r = real(fd, offset, whence); FINISH(#name, 0, r); return r; \
}
SEEK_WRAPPER(lseek, off_t)
SEEK_WRAPPER(lseek64, off64_t)

int statx(int fd, const char *path, int flags, unsigned int mask, struct statx *buf) {
    static _Thread_local int (*real)(int, const char *, int, unsigned int, struct statx *);
    if (!real) real = dlsym(RTLD_NEXT, "statx");
    if (!active || !(path_selected(path) || (!path[0] && fd_selected(fd))))
        return real(fd, path, flags, mask, buf);
    START; int r = real(fd, path, flags, mask, buf); FINISH("statx", 0, r); return r;
}
int lstat64(const char *path, struct stat64 *buf) {
    static _Thread_local int (*real)(const char *, struct stat64 *);
    if (!real) real = dlsym(RTLD_NEXT, "lstat64");
    if (!active || !path_selected(path)) return real(path, buf);
    START; int r = real(path, buf); FINISH("lstat64", 0, r); return r;
}
int fstat64(int fd, struct stat64 *buf) {
    static _Thread_local int (*real)(int, struct stat64 *);
    if (!real) real = dlsym(RTLD_NEXT, "fstat64");
    if (!active || !fd_selected(fd)) return real(fd, buf);
    START; int r = real(fd, buf); FINISH("fstat64", 0, r); return r;
}
ssize_t readlink(const char *path, char *buf, size_t n) {
    static _Thread_local ssize_t (*real)(const char *, char *, size_t);
    if (!real) real = dlsym(RTLD_NEXT, "readlink");
    if (!active || !path_selected(path)) return real(path, buf, n);
    START; ssize_t r = real(path, buf, n); FINISH("readlink", n, r); return r;
}
int close(int fd) {
    static _Thread_local int (*real)(int);
    if (!real) real = dlsym(RTLD_NEXT, "close");
    int selected = active && fd_selected(fd);
    track(fd, 0);
    if (!selected) return real(fd);
    START; int r = real(fd); FINISH("close", 0, r); return r;
}
