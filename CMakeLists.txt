cmake_minimum_required(VERSION 3.15)

include(proj.conf)

# One version triplet drives APP_VER_X/Y/Z, the HID 0xF8 response, OTA
# STATUS, and the SDK-generated EVENT_Vx.y.z header.  Release automation may
# pass either -DFIRMWARE_VERSION=x.y.z / FIRMWARE_VERSION in the environment,
# or the SDK-native PROJECT_SDK_VERSION equivalent.
set(DS5_SELECTED_VERSION "")
if(DEFINED FIRMWARE_VERSION AND NOT "${FIRMWARE_VERSION}" STREQUAL "")
    set(DS5_SELECTED_VERSION "${FIRMWARE_VERSION}")
elseif(DEFINED ENV{FIRMWARE_VERSION} AND
       NOT "$ENV{FIRMWARE_VERSION}" STREQUAL "")
    set(DS5_SELECTED_VERSION "$ENV{FIRMWARE_VERSION}")
elseif(DEFINED PROJECT_SDK_VERSION AND
       NOT "${PROJECT_SDK_VERSION}" STREQUAL "")
    set(DS5_SELECTED_VERSION "${PROJECT_SDK_VERSION}")
elseif(DEFINED ENV{PROJECT_SDK_VERSION} AND
       NOT "$ENV{PROJECT_SDK_VERSION}" STREQUAL "")
    set(DS5_SELECTED_VERSION "$ENV{PROJECT_SDK_VERSION}")
else()
    set(DS5_SELECTED_VERSION "3.5.0")
endif()
if(NOT DS5_SELECTED_VERSION MATCHES
   "^([0-9]|[1-9][0-9]|1[0-9][0-9]|2[0-4][0-9]|25[0-4])\\.([0-9]|[1-9][0-9]|1[0-9][0-9]|2[0-4][0-9]|25[0-4])\\.([0-9]|[1-9][0-9]|1[0-9][0-9]|2[0-4][0-9]|25[0-4])$")
    message(FATAL_ERROR
        "Firmware version must be an x.y.z triplet with each value 0..254: ${DS5_SELECTED_VERSION}")
endif()
string(LENGTH "${DS5_SELECTED_VERSION}" DS5_SELECTED_VERSION_LENGTH)
if(DS5_SELECTED_VERSION_LENGTH GREATER 8)
    message(FATAL_ERROR
        "Firmware version must fit the BL60X EVENT_V software header (x.y.z <= 8 bytes): ${DS5_SELECTED_VERSION}")
endif()
set(PROJECT_SDK_VERSION "${DS5_SELECTED_VERSION}" CACHE STRING
    "DS5Dongle firmware version" FORCE)
message(STATUS "DS5Dongle firmware version: ${PROJECT_SDK_VERSION}")

# The custom logger in src/debug_log.h is intentionally independent of the
# Bouffalo SDK's CONFIG_LOG_LEVEL.  Keep production at lifecycle/event level
# by default and make diagnostic builds opt in explicitly with
# -DDS5_LOG_LEVEL=3.  Reject everything outside the documented 0..3 contract
# instead of silently falling back to a different preprocessor value.
if(NOT DEFINED DS5_LOG_LEVEL)
    if(DEFINED ENV{DS5_LOG_LEVEL} AND
       NOT "$ENV{DS5_LOG_LEVEL}" STREQUAL "")
        set(DS5_LOG_LEVEL "$ENV{DS5_LOG_LEVEL}" CACHE STRING
            "DS5Dongle custom log level (0..3)")
    else()
        set(DS5_LOG_LEVEL "2" CACHE STRING
            "DS5Dongle custom log level (0..3)")
    endif()
endif()
set_property(CACHE DS5_LOG_LEVEL PROPERTY STRINGS 0 1 2 3)
if(NOT "${DS5_LOG_LEVEL}" MATCHES "^[0-3]$")
    message(FATAL_ERROR
        "DS5_LOG_LEVEL must be exactly one of 0, 1, 2, or 3: ${DS5_LOG_LEVEL}")
endif()
message(STATUS "DS5Dongle custom log level: ${DS5_LOG_LEVEL}")

# A release public key is supplied as a 65-byte SEC1 uncompressed P-256 point
# (04 || X || Y).  Only X || Y enters the generated build header.  No default
# or test key is embedded: a normal build remains flashable but OTA fails
# closed and reports KEY_CONFIGURED=0 until a release key is provided.
set(DS5_OTA_KEY_FILE "")
if(DEFINED DS5_OTA_PUBLIC_KEY_FILE AND
   NOT "${DS5_OTA_PUBLIC_KEY_FILE}" STREQUAL "")
    set(DS5_OTA_KEY_FILE "${DS5_OTA_PUBLIC_KEY_FILE}")
elseif(DEFINED ENV{DS5_OTA_PUBLIC_KEY_FILE} AND
       NOT "$ENV{DS5_OTA_PUBLIC_KEY_FILE}" STREQUAL "")
    set(DS5_OTA_KEY_FILE "$ENV{DS5_OTA_PUBLIC_KEY_FILE}")
endif()

set(DS5_OTA_GENERATED_DIR "${CMAKE_CURRENT_BINARY_DIR}/generated/ds5_ota")
file(MAKE_DIRECTORY "${DS5_OTA_GENERATED_DIR}")
set(DS5_OTA_KEY_HEADER
    "${DS5_OTA_GENERATED_DIR}/ota_release_public_key.h")

if(NOT DS5_OTA_KEY_FILE STREQUAL "")
    get_filename_component(DS5_OTA_KEY_FILE "${DS5_OTA_KEY_FILE}"
                           ABSOLUTE BASE_DIR "${CMAKE_CURRENT_LIST_DIR}")
    if(NOT EXISTS "${DS5_OTA_KEY_FILE}")
        message(FATAL_ERROR
            "DS5_OTA_PUBLIC_KEY_FILE does not exist: ${DS5_OTA_KEY_FILE}")
    endif()
    file(SIZE "${DS5_OTA_KEY_FILE}" DS5_OTA_KEY_SIZE)
    if(NOT DS5_OTA_KEY_SIZE EQUAL 65)
        message(FATAL_ERROR
            "DS5_OTA_PUBLIC_KEY_FILE must be exactly 65-byte SEC1 uncompressed P-256")
    endif()
    file(READ "${DS5_OTA_KEY_FILE}" DS5_OTA_KEY_HEX HEX)
    string(TOLOWER "${DS5_OTA_KEY_HEX}" DS5_OTA_KEY_HEX)
    string(SUBSTRING "${DS5_OTA_KEY_HEX}" 0 2 DS5_OTA_KEY_PREFIX)
    if(NOT DS5_OTA_KEY_PREFIX STREQUAL "04")
        message(FATAL_ERROR
            "DS5_OTA_PUBLIC_KEY_FILE must start with SEC1 uncompressed marker 0x04")
    endif()
    string(SUBSTRING "${DS5_OTA_KEY_HEX}" 2 128 DS5_OTA_KEY_XY_HEX)
    string(REGEX REPLACE "([0-9a-f][0-9a-f])" "0x\\1, "
           DS5_OTA_KEY_INITIALIZER "${DS5_OTA_KEY_XY_HEX}")
    file(WRITE "${DS5_OTA_KEY_HEADER}"
        "#ifndef OTA_RELEASE_PUBLIC_KEY_H\n"
        "#define OTA_RELEASE_PUBLIC_KEY_H\n"
        "#define DS5_OTA_RELEASE_PUBLIC_KEY_CONFIGURED 1\n"
        "#define DS5_OTA_RELEASE_PUBLIC_KEY_INITIALIZER { ${DS5_OTA_KEY_INITIALIZER} }\n"
        "#endif\n")
    message(STATUS "OTA release public key: configured")
else()
    file(WRITE "${DS5_OTA_KEY_HEADER}"
        "#ifndef OTA_RELEASE_PUBLIC_KEY_H\n"
        "#define OTA_RELEASE_PUBLIC_KEY_H\n"
        "#define DS5_OTA_RELEASE_PUBLIC_KEY_CONFIGURED 0\n"
        "#define DS5_OTA_RELEASE_PUBLIC_KEY_INITIALIZER { 0 }\n"
        "#endif\n")
    message(WARNING
        "OTA release public key is not configured; production OTA will fail closed")
endif()

# FORCE_FS=1 env var → remove CONFIG_USB_HS so SDK sets FORCE_FS bit
if(DEFINED ENV{FORCE_FS})
    unset(CONFIG_USB_HS)
endif()

# Ai-M61-32S carries 4 MiB of x8 PSRAM.  Enable it only for that board: the
# other supported BL616 boards are not guaranteed to have external RAM.
# Keep PSRAM out of the generic allocator so latency-sensitive USB, Bluetooth,
# and Opus allocations cannot migrate there implicitly.  Application objects
# must opt in explicitly through DS5_PSRAM_COLD_DATA.
set(DS5_TARGET_HAS_PSRAM OFF)
if(NOT DEFINED ENV{BOARD_LCTECH_616} AND
   NOT DEFINED ENV{BOARD_M0S_DOCK})
    set(CONFIG_PSRAM y)
    set(CONFIG_PSRAM_LENGTH 4194304)
    set(CONFIG_PSRAM_SKIP_REGISTER_HEAP y)
    set(CONFIG_PSRAM_COPY_CODE n)
    set(DS5_TARGET_HAS_PSRAM ON)
endif()

find_package(bouffalo_sdk REQUIRED HINTS $ENV{BL_SDK_BASE})

sdk_set_main_file(src/main.c)

# These legacy SDK switches are consumed with C preprocessor guards but are
# not declared Kconfig symbols, so defconfig alone does not emit -D flags.
# Scope them to the only targets that consume them.
target_compile_definitions(app PRIVATE CONFIG_ENABLE_IMG_HASH)
target_compile_definitions(app PRIVATE LOG_LEVEL=${DS5_LOG_LEVEL})
if(TARGET libfota)
    target_compile_definitions(libfota PRIVATE CONFIG_OTA_VERSION_CHECK)
else()
    message(FATAL_ERROR "CONFIG_FOTA enabled but libfota target is missing")
endif()

target_sources(app PRIVATE
    src/bt_hid_host.c
    src/ds5_protocol.c
    src/usb_gamepad.c
    src/ds5_usb_audio.c
    src/audio.c
    src/usb_wake.c
    src/led_status.c
    src/state_mgr.c
    src/config.c
    src/dse.c
    src/remap.c
    src/ota_update.c
    src/runtime_diag.c
)

sdk_add_include_directories(src)
sdk_add_include_directories(${BL_SDK_BASE}/components/wireless/bluetooth/blestack/src/include)
sdk_add_include_directories(${BL_SDK_BASE}/components/wireless/bluetooth/btprofile/include)
sdk_add_include_directories(${BL_SDK_BASE}/components/utils/partition)
sdk_add_include_directories(${BL_SDK_BASE}/components/fota/ota)

# Opus codec (fixed-point, encoder only)
include(lib/opus.cmake)
target_sources(app PRIVATE ${OPUS_SOURCES})
option(DS5_OPUS_E907_OPTIMIZATIONS
    "Use bit-exact E907 DSP and power-of-two division fast paths" ON)
if(DS5_OPUS_E907_OPTIMIZATIONS)
    target_compile_definitions(app PRIVATE
        M61_OPUS_E907_CLZ32=1
        M61_OPUS_E907_Q15_KMMWB2=1
        M61_OPUS_E907_Q16_SMMWB=1
        M61_OPUS_E907_POW2_DIV=1
    )
    message(STATUS "Opus E907 bit-exact fast paths: enabled")
else()
    message(STATUS "Opus E907 bit-exact fast paths: disabled")
endif()
target_include_directories(app PRIVATE
    ${DS5_OTA_GENERATED_DIR}
    ${CMAKE_CURRENT_LIST_DIR}/lib/opus/include
    ${CMAKE_CURRENT_LIST_DIR}/lib/opus/celt
    ${CMAKE_CURRENT_LIST_DIR}/lib/opus/silk
    ${CMAKE_CURRENT_LIST_DIR}/lib/opus/silk/fixed
    ${CMAKE_CURRENT_LIST_DIR}/lib/opus
)
# Board selection via env vars (mutually exclusive)
if(DEFINED ENV{BOARD_LCTECH_616})
    target_compile_definitions(app PRIVATE BOARD_LCTECH_616=1)
    message(STATUS "Target board: LCTech BL616")
elseif(DEFINED ENV{BOARD_M0S_DOCK})
    target_compile_definitions(app PRIVATE BOARD_M0S_DOCK=1)
    message(STATUS "Target board: Sipeed M0S Dock (BL616)")
else()
    message(STATUS "Target board: Ai-M61-32S-Kit (BL618)")
endif()

if(DS5_TARGET_HAS_PSRAM)
    message(STATUS "Memory policy: 4 MiB PSRAM enabled for explicit cold data")
endif()

target_compile_definitions(app PRIVATE HAVE_CONFIG_H)
if(DEFINED ENV{DS5_OTA_ALLOW_UNSIGNED_DEV} AND
   "$ENV{DS5_OTA_ALLOW_UNSIGNED_DEV}" STREQUAL "1")
    target_compile_definitions(app PRIVATE DS5_OTA_ALLOW_UNSIGNED_DEV=1)
    message(WARNING "UNSAFE development OTA: unsigned images are accepted")
endif()
if(DEFINED ENV{FORCE_FS})
    target_compile_definitions(app PRIVATE FORCE_FS_MODE=1)
    message(STATUS "USB mode: Full-Speed (compatibility variant)")
else()
    message(STATUS "USB mode: High-Speed (default)")
endif()
target_compile_options(app PRIVATE
    $<$<COMPILE_LANGUAGE:C>:-include${CMAKE_CURRENT_LIST_DIR}/lib/opus_config.h>
)

# The Bouffalo SDK defaults to -Os.  Keep that conservative setting for the
# USB/BT control paths, but allow the two measured DSP hot spots to trade a
# small amount of the ample flash budget for lower worst-case execution time.
# Do not enable application-wide LTO: the Bouffalo post-build DWARF-CFI tool
# extracts libapp.a with an ar invocation that cannot consume LTO bytecode.
# Source-level options are emitted after the SDK defaults, so these override
# -Os only for the selected translation units.
set_source_files_properties(src/audio.c PROPERTIES COMPILE_OPTIONS "-O3")
set_source_files_properties(${OPUS_SOURCES} PROPERTIES COMPILE_OPTIONS "-O2")

project(ds5dongle_bl618)
