// SPDX-License-Identifier: GPL-3.0-or-later
// anonless: lets the amdgpu kernel driver import shared memory (Wine sections) as GPU memory.
//
// libdrm's amdgpu_create_bo_from_user_mem always passes AMDGPU_GEM_USERPTR_ANONONLY, so the kernel
// refuses a host pointer that is not private anonymous memory. Under Wine/Proton the title's
// physical memory is a section (a shared mapping): RADV's VK_EXT_external_memory_host import of it
// fails, and VIctory's in-place heaps (renderer patch 1, VICTORY_PORT_CONSTANTS) were other memory.
// This preload clears the bit on that one ioctl; every other call goes to libdrm untouched.
// The kernel accepts shared pages without the bit. Not an official interface: a workaround.
//
// Build:  gcc -O2 -shared -fPIC -o anonless.so anonless.c -ldl $(pkg-config --cflags libdrm)
// Use:    LD_PRELOAD=/path/to/anonless.so  (tools/linux/play.sh builds and preloads it)
// Debug:  ANONLESS_DEBUG=1 logs each import it changed.
#define _GNU_SOURCE
#include <dlfcn.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <amdgpu_drm.h>

int drmCommandWriteRead(int fd, unsigned long index, void *data, unsigned long size) {
    static int (*real)(int, unsigned long, void *, unsigned long);
    if (!real) {
        void *libdrm = dlopen("libdrm.so.2", RTLD_LAZY);
        real = libdrm ? (int (*)(int, unsigned long, void *, unsigned long))dlsym(libdrm, "drmCommandWriteRead") : NULL;
        if (!real) {
            fprintf(stderr, "anonless: libdrm.so.2's drmCommandWriteRead not found\n");
            abort();
        }
    }
    if (index == DRM_AMDGPU_GEM_USERPTR && size >= sizeof(struct drm_amdgpu_gem_userptr)) {
        struct drm_amdgpu_gem_userptr *userptr = data;
        userptr->flags &= ~(uint32_t)AMDGPU_GEM_USERPTR_ANONONLY;
        int result = real(fd, index, data, size);
        if (getenv("ANONLESS_DEBUG")) {
            fprintf(stderr, "anonless: userptr %#llx+%#llx flags %#x -> %d\n",
                    (unsigned long long)userptr->addr, (unsigned long long)userptr->size, userptr->flags, result);
        }
        return result;
    }
    return real(fd, index, data, size);
}
