#ifndef FOLIOFORGE_FOLIO_FFI_H
#define FOLIOFORGE_FOLIO_FFI_H

#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct FolioResult {
    int32_t code;
    char *json;
} FolioResult;

typedef struct FolioCancellation FolioCancellation;

typedef void (*FolioProgressCallback)(const char *event_json, void *user_data);

const char *folio_version(void);
char *folio_capabilities(void);

FolioResult *folio_convert(const char *request_json);
FolioResult *folio_convert_with_cancellation(
    const char *request_json,
    const FolioCancellation *cancellation);
FolioResult *folio_convert_with_progress(
    const char *request_json,
    const FolioCancellation *cancellation,
    FolioProgressCallback callback,
    void *user_data);
FolioResult *folio_batch_list_directory(const char *path);
FolioResult *folio_batch_convert_with_progress(
    const char *request_json,
    const FolioCancellation *cancellation,
    FolioProgressCallback callback,
    void *user_data);

FolioResult *folio_analyze(const char *request_json);
FolioResult *folio_preview(const char *request_json);

FolioResult *folio_inspect(const char *path);
FolioResult *folio_validate(const char *path);

FolioCancellation *folio_cancellation_new(void);
void folio_cancellation_cancel(FolioCancellation *cancellation);
void folio_cancellation_free(FolioCancellation *cancellation);

void folio_result_free(FolioResult *result);
void folio_string_free(char *value);

#ifdef __cplusplus
}
#endif

#endif
