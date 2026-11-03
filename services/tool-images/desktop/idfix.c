/* Report "already that user" as success.
 *
 * The X server starts xkbcomp through a helper that first calls
 * setgid(getgid()) and setuid(getuid()): drop privileges to the identity it
 * already has, which does nothing. A gateway sandbox's syscall filter refuses
 * every setuid-family call with EPERM, even to the same id, and the helper treats
 * that as fatal, so Xtigervnc died with "Failed to compile keymap".
 *
 * Preloaded for the desktop only. A call that would change the identity is passed
 * on to libc and still refused; nothing here grants a privilege.
 */
#define _GNU_SOURCE
#include <dlfcn.h>
#include <sys/types.h>
#include <unistd.h>

int setuid(uid_t uid) {
    if (uid == getuid()) return 0;
    int (*next)(uid_t) = dlsym(RTLD_NEXT, "setuid");
    return next(uid);
}

int setgid(gid_t gid) {
    if (gid == getgid()) return 0;
    int (*next)(gid_t) = dlsym(RTLD_NEXT, "setgid");
    return next(gid);
}
