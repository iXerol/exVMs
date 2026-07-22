#ifndef EXVMS_H
#define EXVMS_H

#include <stdint.h>
#include <stddef.h>
#include <CoreFoundation/CFBase.h>
#include <Foundation/NSObjCRuntime.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef NS_ENUM(int32_t, ExvmsStatusCode) {
    ExvmsStatusCodeOK = 0,
    ExvmsStatusCodeError = 1,
    ExvmsStatusCodeInvalidArgument = 2,
    ExvmsStatusCodePanic = 255,
} NS_SWIFT_NAME(ExvmsStatusCode);

typedef NS_ENUM(int32_t, ExvmsStoreKind) {
    ExvmsStoreKindCloudRecordings = 0,
    ExvmsStoreKindLocalRecordings = 1,
} NS_SWIFT_NAME(ExvmsStoreKind);

typedef NS_ENUM(int32_t, ExvmsPathKind) {
    ExvmsPathKindRelative = 0,
    ExvmsPathKindAbsolute = 1,
    ExvmsPathKindMissing = 2,
} NS_SWIFT_NAME(ExvmsPathKind);

typedef NS_ENUM(int32_t, ExvmsAvailability) {
    ExvmsAvailabilityInRecordingsDirectory = 0,
    ExvmsAvailabilityNeedsDownload = 1,
    ExvmsAvailabilityOutsideRecordingsDirectory = 2,
    ExvmsAvailabilityCorrupted = 3,
    ExvmsAvailabilityChecking = 4,
} NS_SWIFT_NAME(ExvmsAvailability);

typedef NS_OPTIONS(uint32_t, ExvmsExportGrouping) {
    ExvmsExportGroupingFolder = 1u << 0,
    ExvmsExportGroupingDate = 1u << 1,
} NS_SWIFT_NAME(ExvmsExportGrouping);

typedef struct ExvmsStoreResult {
    ExvmsStatusCode code;
    char *_Nullable message;
    char *_Nullable recordings_dir;
    char *_Nullable db_path;
} ExvmsStoreResult;

typedef struct ExvmsRecording {
    int64_t id;
    char *_Nullable title;
    double recorded_at;
    double duration;
    double local_duration;
    int32_t is_deleted;
    char *_Nullable db_path;
    char *_Nullable folder_name;
    ExvmsStoreKind store_kind;
    ExvmsPathKind path_kind;
    ExvmsAvailability availability;
    char *_Nullable availability_detail;
} ExvmsRecording;

typedef struct ExvmsListResult {
    ExvmsStatusCode code;
    char *_Nullable message;
    char *_Nullable recordings_dir;
    char *_Nullable db_path;
    ExvmsRecording *_Nullable recordings;
    uintptr_t recordings_len;
} ExvmsListResult;

typedef struct ExvmsExportSuccess {
    int64_t id;
    char *_Nullable target;
} ExvmsExportSuccess;

typedef struct ExvmsExportSkipped {
    int64_t id;
    char *_Nullable reason;
} ExvmsExportSkipped;

typedef struct ExvmsExportFailure {
    int64_t id;
    char *_Nullable error;
} ExvmsExportFailure;

typedef struct ExvmsExportResult {
    ExvmsStatusCode code;
    char *_Nullable message;
    ExvmsExportSuccess *_Nullable successes;
    uintptr_t successes_len;
    ExvmsExportSkipped *_Nullable skipped;
    uintptr_t skipped_len;
    ExvmsExportFailure *_Nullable failures;
    uintptr_t failures_len;
} ExvmsExportResult;

typedef struct ExvmsCancelToken ExvmsCancelToken;

ExvmsStoreResult exvms_discover_store(void)
    NS_SWIFT_NAME(exvmsDiscoverStore());
ExvmsListResult exvms_list_recordings(void)
    NS_SWIFT_NAME(exvmsListRecordings());
ExvmsExportResult exvms_export_recordings(
    const char *_Nonnull output_dir,
    const int64_t *_Nonnull ids,
    uintptr_t ids_len,
    uintptr_t jobs,
    const char *_Nullable filename_template,
    ExvmsExportGrouping export_grouping
) NS_SWIFT_NAME(exvmsExportRecordings(outputDirectory:ids:idsCount:jobs:filenameTemplate:exportGrouping:));
ExvmsExportResult exvms_export_recordings_with_cancel(
    const char *_Nonnull output_dir,
    const int64_t *_Nonnull ids,
    uintptr_t ids_len,
    uintptr_t jobs,
    const char *_Nullable filename_template,
    ExvmsExportGrouping export_grouping,
    const ExvmsCancelToken *_Nullable cancel_token
) NS_SWIFT_NAME(exvmsExportRecordings(outputDirectory:ids:idsCount:jobs:filenameTemplate:exportGrouping:cancelToken:));

ExvmsCancelToken *_Nonnull exvms_cancel_token_create(void)
    NS_SWIFT_NAME(exvmsCancelTokenCreate());
void exvms_cancel_token_cancel(const ExvmsCancelToken *_Nullable token)
    NS_SWIFT_NAME(exvmsCancelTokenCancel(_:));
void exvms_cancel_token_free(ExvmsCancelToken *_Nullable token)
    NS_SWIFT_NAME(exvmsCancelTokenFree(_:));

void exvms_free_string(char *_Nullable value)
    NS_SWIFT_NAME(exvmsFreeString(_:));
void exvms_free_store_result(ExvmsStoreResult result)
    NS_SWIFT_NAME(exvmsFreeStoreResult(_:));
void exvms_free_list_result(ExvmsListResult result)
    NS_SWIFT_NAME(exvmsFreeListResult(_:));
void exvms_free_export_result(ExvmsExportResult result)
    NS_SWIFT_NAME(exvmsFreeExportResult(_:));

#ifdef __cplusplus
}
#endif

#endif
