#define _DARWIN_C_SOURCE
#include <dlfcn.h>
#include <errno.h>
#include <inttypes.h>
#include <mach/mach.h>
#include <mach/mach_vm.h>
#include <mach-o/dyld.h>
#include <mach-o/loader.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/sysctl.h>
#include <unistd.h>
#if __has_include(<ptrauth.h>)
#include <ptrauth.h>
#endif

/* Read only this process's mappings. Target symbols are resolved, never called. */
enum { MAX_FUNCTION = 32768, MAX_STARTS = 1048576, MAX_CAPTURES = 64 };
static size_t captured_bytes;
static unsigned capture_count;
static unsigned pointer_count;

typedef struct {
    Dl_info symbol;
    const struct mach_header_64 *header;
    intptr_t slide;
    uint64_t text_base, code_start, code_end, linkedit_base;
    uint32_t starts_offset, starts_size, current_version;
    char uuid[33];
} Image;

static void quoted(const char *s) {
    putchar('"');
    if (s) for (const unsigned char *p = (const unsigned char *)s; *p; ++p) {
        if (*p == '"' || *p == '\\') { putchar('\\'); putchar(*p); }
        else if (*p >= 32 && *p < 127) putchar(*p);
        else printf("\\u%04x", *p);
    }
    putchar('"');
}

static void hex(const unsigned char *data, size_t size) {
    for (size_t i = 0; i < size; ++i) printf("%02x", data[i]);
}

static void failure(const char *operation, const char *reason, int code) {
    printf("{\"event\":\"capture_error\",\"operation\":");
    quoted(operation);
    printf(",\"reason\":");
    quoted(reason);
    printf(",\"code\":%d}\n", code);
    fflush(stdout);
}

static uintptr_t code_address(void *pointer) {
#if __has_feature(ptrauth_calls)
    return (uintptr_t)ptrauth_strip(pointer, ptrauth_key_function_pointer);
#else
    return (uintptr_t)pointer;
#endif
}

static int own_read(uint64_t address, void *buffer, size_t size) {
    mach_vm_size_t copied = 0;
    kern_return_t rc = mach_vm_read_overwrite(mach_task_self(), address, size,
                                             (mach_vm_address_t)buffer, &copied);
    return rc == KERN_SUCCESS && copied == size;
}

static int image_at(uint64_t address, Image *out) {
    memset(out, 0, sizeof(*out));
    if (!dladdr((void *)(uintptr_t)address, &out->symbol)) return 0;
    out->header = out->symbol.dli_fbase;
    if (out->header->magic != MH_MAGIC_64 || out->header->sizeofcmds > MAX_STARTS) return 0;
    int found = 0;
    for (uint32_t i = 0; i < _dyld_image_count(); ++i) {
        if ((const void *)_dyld_get_image_header(i) == (const void *)out->header) {
            out->slide = _dyld_get_image_vmaddr_slide(i);
            found = 1;
            break;
        }
    }
    if (!found) return 0;
    const unsigned char *cursor = (const unsigned char *)(out->header + 1);
    const unsigned char *end = cursor + out->header->sizeofcmds;
    for (uint32_t i = 0; i < out->header->ncmds; ++i) {
        if (cursor + sizeof(struct load_command) > end) return 0;
        const struct load_command *command = (const struct load_command *)cursor;
        if (command->cmdsize < sizeof(*command) || cursor + command->cmdsize > end) return 0;
        if (command->cmd == LC_UUID && command->cmdsize >= sizeof(struct uuid_command)) {
            const struct uuid_command *uuid = (const struct uuid_command *)command;
            for (int b = 0; b < 16; ++b) snprintf(out->uuid + b * 2, 3, "%02x", uuid->uuid[b]);
        } else if (command->cmd == LC_ID_DYLIB &&
                   command->cmdsize >= sizeof(struct dylib_command)) {
            out->current_version = ((const struct dylib_command *)command)->dylib.current_version;
        } else if (command->cmd == LC_FUNCTION_STARTS &&
                   command->cmdsize >= sizeof(struct linkedit_data_command)) {
            const struct linkedit_data_command *starts = (const struct linkedit_data_command *)command;
            out->starts_offset = starts->dataoff;
            out->starts_size = starts->datasize;
        } else if (command->cmd == LC_SEGMENT_64 &&
                   command->cmdsize >= sizeof(struct segment_command_64)) {
            const struct segment_command_64 *segment = (const struct segment_command_64 *)command;
            uint64_t runtime = segment->vmaddr + out->slide;
            if (!strncmp(segment->segname, "__TEXT", 16)) out->text_base = runtime;
            if (!strncmp(segment->segname, "__LINKEDIT", 16))
                out->linkedit_base = runtime - segment->fileoff;
            if ((segment->initprot & VM_PROT_EXECUTE) && address >= runtime &&
                address - runtime < segment->vmsize) {
                out->code_start = runtime;
                out->code_end = runtime + segment->vmsize;
            }
        }
        cursor += command->cmdsize;
    }
    return out->uuid[0] && out->text_base && out->code_end > out->code_start;
}

static void image_fields(const Image *image, uint64_t address) {
    printf("\"address\":\"0x%" PRIx64 "\",\"image\":", address);
    quoted(image->symbol.dli_fname);
    printf(",\"image_uuid\":\"%s\",\"current_version_packed\":%u,"
           "\"header_address\":\"0x%" PRIxPTR "\",\"slide\":\"0x%" PRIxPTR "\","
           "\"unslid_address\":\"0x%" PRIx64 "\",\"text_base\":\"0x%" PRIx64 "\","
           "\"executable_mapping_start\":\"0x%" PRIx64 "\","
           "\"executable_mapping_end\":\"0x%" PRIx64 "\",\"nearest_symbol\":",
           image->uuid, image->current_version, (uintptr_t)image->header, (uintptr_t)image->slide,
           address - image->slide, image->text_base, image->code_start, image->code_end);
    quoted(image->symbol.dli_sname);
    printf(",\"nearest_symbol_address\":\"0x%" PRIxPTR "\"",
           code_address(image->symbol.dli_saddr));
}

static void sysctl_record(const char *name) {
    unsigned char data[4096] = {0};
    size_t size = sizeof(data);
    errno = 0;
    int rc = sysctlbyname(name, data, &size, NULL, 0);
    int error = errno;
    printf("{\"event\":\"live_sysctl\",\"name\":");
    quoted(name);
    printf(",\"rc\":%d,\"errno\":%d,\"returned_size\":%zu,\"raw_hex\":\"", rc, error, size);
    if (rc == 0 && size <= sizeof(data)) hex(data, size);
    printf("\",\"text\":");
    if (rc == 0 && size > 0 && size <= sizeof(data) && memchr(data, 0, size))
        quoted((const char *)data);
    else printf("null");
    printf("}\n");
}

static void symbol_metadata(const char *name) {
    void *pointer = dlsym(RTLD_DEFAULT, name);
    Image image;
    uint64_t address = code_address(pointer);
    if (!pointer || !image_at(address, &image)) {
        failure(name, "symbol_or_executable_image_unavailable", 0);
        return;
    }
    printf("{\"event\":\"symbol_metadata\",\"symbol\":");
    quoted(name);
    printf(",");
    image_fields(&image, address);
    printf("}\n");
}

static int function_extent(const Image *image, uint64_t address, uint64_t *start,
                           uint64_t *end, uint64_t *table_address) {
    if (!image->linkedit_base || !image->starts_size || image->starts_size > MAX_STARTS) return 0;
    unsigned char *data = malloc(image->starts_size);
    if (!data) return 0;
    *table_address = image->linkedit_base + image->starts_offset;
    if (!own_read(*table_address, data, image->starts_size)) { free(data); return 0; }
    uint64_t position = image->text_base, previous = 0;
    size_t cursor = 0;
    int found = 0;
    while (cursor < image->starts_size) {
        uint64_t delta = 0;
        unsigned shift = 0;
        unsigned char byte;
        do {
            if (cursor >= image->starts_size || shift >= 63) goto done;
            byte = data[cursor++];
            delta |= (uint64_t)(byte & 127) << shift;
            shift += 7;
        } while (byte & 128);
        if (!delta || position > UINT64_MAX - delta) break;
        position += delta;
        if (position > address) {
            if (previous && previous <= address && previous >= image->code_start &&
                position <= image->code_end) {
                *start = previous;
                *end = position;
                found = 1;
            }
            break;
        }
        previous = position;
    }
done:
    free(data);
    return found;
}

static void capture(uint64_t address) {
    if (++capture_count > MAX_CAPTURES) { failure("capture", "function_count_cap", 0); return; }
    Image image;
    if (!image_at(address, &image)) { failure("capture", "address_not_in_own_executable_image", 0); return; }
    uint64_t start = address, end = address + 256, table = 0;
    int complete_extent = function_extent(&image, address, &start, &end, &table);
    if (end > image.code_end) end = image.code_end;
    uint64_t original_end = end;
    if (end - start > MAX_FUNCTION) end = start + MAX_FUNCTION;
    size_t size = (size_t)(end - start);
    if (!size || captured_bytes + size > MAX_CAPTURES * MAX_FUNCTION) {
        failure("capture", "byte_budget_or_extent_invalid", 0); return;
    }
    unsigned char *bytes = malloc(size);
    if (!bytes || !own_read(start, bytes, size)) {
        free(bytes); failure("capture", "own_mapping_read_failed", 0); return;
    }
    captured_bytes += size;
    printf("{\"event\":\"function_capture\",");
    image_fields(&image, address);
    printf(",\"start\":\"0x%" PRIx64 "\",\"end\":\"0x%" PRIx64 "\","
           "\"derived_end\":\"0x%" PRIx64 "\",\"requested_offset\":%" PRIu64 ","
           "\"extent_derivation\":\"%s\",\"full_derived_extent_captured\":%s,"
           "\"function_starts_table_address\":\"0x%" PRIx64 "\","
           "\"function_starts_table_size\":%u,\"byte_count\":%zu,\"bytes_hex\":\"",
           start, end, original_end, address - start,
           complete_extent ? "LC_FUNCTION_STARTS_adjacent_entries" : "bounded_window_extent_unknown",
           complete_extent && end == original_end ? "true" : "false",
           table, image.starts_size, size);
    hex(bytes, size);
    printf("\"}\n");
    free(bytes);
}

static void pointer_read(uint64_t address) {
    if (++pointer_count > 128) { failure("pointer", "pointer_count_cap", 0); return; }
    uintptr_t pointer;
    if (!own_read(address, &pointer, sizeof(pointer))) {
        failure("pointer", "own_mapping_pointer_unreadable", 0); return;
    }
    printf("{\"event\":\"pointer_read\",\"address\":\"0x%" PRIx64 "\","
           "\"raw_value\":\"0x%" PRIxPTR "\",\"code_value\":\"0x%" PRIxPTR "\","
           "\"pointer_auth_strip_applied\":%s}\n", address, pointer, code_address((void *)pointer),
#if __has_feature(ptrauth_calls)
           "true"
#else
           "false"
#endif
    );
}

int main(void) {
    if (getuid() == 0) return 1;
    alarm(420);
    setvbuf(stdout, NULL, _IOLBF, 0);
    printf("{\"event\":\"capture_process\",\"pid\":%d,\"uid\":%d,\"pointer_size\":%zu,"
           "\"target_operations_invoked\":false,\"function_limit\":%d,\"bytes_per_function_limit\":%d}\n",
           getpid(), getuid(), sizeof(void *), MAX_CAPTURES, MAX_FUNCTION);
    const char *queries[] = {"kern.uuid", "kern.version", "kern.osrelease", "kern.osversion",
                             "kern.osproductversion", "kern.bootsessionuuid"};
    for (size_t i = 0; i < sizeof(queries) / sizeof(queries[0]); ++i) sysctl_record(queries[i]);
    const char *symbols[] = {"proc_listpids", "kill", "waitid", "posix_spawn", "execve", "fork", "vfork"};
    for (size_t i = 0; i < sizeof(symbols) / sizeof(symbols[0]); ++i) symbol_metadata(symbols[i]);
    printf("{\"event\":\"metadata_ready\"}\n");
    char line[128];
    while (fgets(line, sizeof(line), stdin)) {
        uint64_t address;
        char extra;
        if (sscanf(line, "capture %" SCNx64 " %c", &address, &extra) == 1) capture(address);
        else if (sscanf(line, "pointer %" SCNx64 " %c", &address, &extra) == 1) pointer_read(address);
        else { failure("protocol", "invalid_request", 0); return 1; }
        fflush(stdout);
    }
    return 0;
}
