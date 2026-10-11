// AppKit activation can be ignored for Wine's unbundled game process. Use
// macOS's public process API, never code or memory inside the client.
#include <Carbon/Carbon.h>
#include <libproc.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

int main(int argc, char **argv) {
    if (argc != 2) return 2;
    char *end;
    long value = strtol(argv[1], &end, 10);
    if (*end || value <= 0 || value > 0x7fffffff) return 2;
    char path[PROC_PIDPATHINFO_MAXSIZE];
    if (proc_pidpath((pid_t)value, path, sizeof(path)) <= 0) return 2;
    char *name = strrchr(path, '/');
    if (!name || strcmp(name + 1, "codoMP_client_shipRetail.exe") != 0) return 2;
    strcpy(name + 1, ".codol-research-copy.json");
    if (access(path, R_OK) != 0) return 2;
    ProcessSerialNumber serial = {0, 0};
    OSStatus status = GetProcessForPID((pid_t)value, &serial);
    if (status == noErr) status = SetFrontProcessWithOptions(&serial, kSetFrontProcessFrontWindowOnly);
    if (status != noErr) { fprintf(stderr, "game activation failed: %d\n", (int)status); return 2; }
    CFRunLoopRunInMode(kCFRunLoopDefaultMode, 0.15, false);
    return 0;
}
