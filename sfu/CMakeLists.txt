cmake_minimum_required(VERSION 3.21)
project(sfu VERSION 1.0.0 LANGUAGES C CXX)

set(CMAKE_CXX_STANDARD 20)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_CXX_EXTENSIONS OFF)

# CMake 4.x drops compatibility for legacy subprojects unless this is set.
# gRPC pulls c-ares with old policy requirements.
if(CMAKE_VERSION VERSION_GREATER_EQUAL "4.0")
    set(CMAKE_POLICY_VERSION_MINIMUM 3.5)
endif()

option(BUILD_SHARED_LIBS "Build shared libraries" OFF)

# Helps IDEs (Ninja): compile_commands.json with correct -I for generated protos.
set(CMAKE_EXPORT_COMPILE_COMMANDS ON)

# -----------------------------------------------------------------------------
# gRPC from git submodule (builds Abseil, protobuf, codegen tools)
# Clone: from repo root, `git submodule update --init --recursive`
# gRPC itself needs recursive submodules — see pipeline in repo docs / chat.
# -----------------------------------------------------------------------------
set(GRPC_ROOT "${CMAKE_CURRENT_SOURCE_DIR}/vendor/grpc")
if(NOT EXISTS "${GRPC_ROOT}/CMakeLists.txt")
    message(FATAL_ERROR
        "gRPC not found at ${GRPC_ROOT}.\n"
        "From repository root run:\n"
        "  git submodule update --init --recursive sfu/vendor/grpc\n")
endif()

set(ABSL_PROPAGATE_CXX_STD ON CACHE BOOL "" FORCE)
set(gRPC_BUILD_TESTS OFF CACHE BOOL "" FORCE)
set(gRPC_BUILD_GRPC_CPP_PLUGIN ON CACHE BOOL "" FORCE)
set(gRPC_BUILD_GRPC_CSHARP_PLUGIN OFF CACHE BOOL "" FORCE)
set(gRPC_BUILD_GRPC_NODE_PLUGIN OFF CACHE BOOL "" FORCE)
set(gRPC_BUILD_GRPC_OBJECTIVE_C_PLUGIN OFF CACHE BOOL "" FORCE)
set(gRPC_BUILD_GRPC_PHP_PLUGIN OFF CACHE BOOL "" FORCE)
set(gRPC_BUILD_GRPC_PYTHON_PLUGIN OFF CACHE BOOL "" FORCE)
set(gRPC_BUILD_GRPC_RUBY_PLUGIN OFF CACHE BOOL "" FORCE)

# gRPC as a subdirectory: Protobuf's install(EXPORT ...) references Abseil targets that are not
# in the same export set; CMake 4.x fails at generate time. We only need to build, not install.
set(gRPC_INSTALL OFF CACHE BOOL "" FORCE)
set(protobuf_INSTALL OFF CACHE BOOL "" FORCE)
set(utf8_range_ENABLE_INSTALL OFF CACHE BOOL "" FORCE)

add_subdirectory("${GRPC_ROOT}" "${CMAKE_CURRENT_BINARY_DIR}/grpc" EXCLUDE_FROM_ALL)

set(_PROTOBUF_LIBPROTOBUF libprotobuf)
set(_REFLECTION grpc++_reflection)
set(_GRPC_GRPCPP grpc++)

# -----------------------------------------------------------------------------
# signaling.proto -> C++ (package sfu; matches tonic::include_proto!(\"sfu\"))
# -----------------------------------------------------------------------------
get_filename_component(SFU_PROTO "${CMAKE_CURRENT_SOURCE_DIR}/../proto/signaling.proto" ABSOLUTE)
get_filename_component(SFU_PROTO_DIR "${SFU_PROTO}" DIRECTORY)
# Next to sources so IntelliSense/VS finds signaling.grpc.pb.h without guessing build dir.
set(PROTO_GEN_DIR "${CMAKE_CURRENT_SOURCE_DIR}/generated")
file(MAKE_DIRECTORY "${PROTO_GEN_DIR}")

set(SFU_PROTO_SRCS "${PROTO_GEN_DIR}/signaling.pb.cc")
set(SFU_PROTO_HDRS "${PROTO_GEN_DIR}/signaling.pb.h")
set(SFU_GRPC_SRCS "${PROTO_GEN_DIR}/signaling.grpc.pb.cc")
set(SFU_GRPC_HDRS "${PROTO_GEN_DIR}/signaling.grpc.pb.h")

add_custom_command(
    OUTPUT "${SFU_PROTO_SRCS}" "${SFU_PROTO_HDRS}" "${SFU_GRPC_SRCS}" "${SFU_GRPC_HDRS}"
    COMMAND protobuf::protoc
    ARGS
        --grpc_out "${PROTO_GEN_DIR}"
        --cpp_out "${PROTO_GEN_DIR}"
        -I "${SFU_PROTO_DIR}"
        --plugin=protoc-gen-grpc=$<TARGET_FILE:grpc_cpp_plugin>
        "${SFU_PROTO}"
    DEPENDS "${SFU_PROTO}" protobuf::protoc grpc_cpp_plugin
    VERBATIM
)

add_library(sfu_grpc_proto STATIC
    "${SFU_GRPC_SRCS}"
    "${SFU_GRPC_HDRS}"
    "${SFU_PROTO_SRCS}"
    "${SFU_PROTO_HDRS}"
)
target_include_directories(sfu_grpc_proto PUBLIC "${PROTO_GEN_DIR}")
set_source_files_properties(
    "${SFU_GRPC_HDRS}" "${SFU_PROTO_HDRS}"
    PROPERTIES GENERATED TRUE
)
target_link_libraries(sfu_grpc_proto PUBLIC
    absl::absl_check
    absl::absl_log
    ${_REFLECTION}
    ${_GRPC_GRPCPP}
    ${_PROTOBUF_LIBPROTOBUF}
)

# -----------------------------------------------------------------------------
# OpenSSL (Windows): prebuilt next to sfu — avoids system install for libSRTP / libjuice.
# Supported layouts:
#   sfu/vendor/openssl/          (bootstrap-openssl.ps1)
#   sfu/vendor/OpenSSL-Win64/    (Shining Light installer / zip default name)
# Download: https://slproweb.com/products/Win32OpenSSL.html — Win64 *full* (not Light).
# -----------------------------------------------------------------------------
set(_SFU_VENDOR_OPENSSL "${CMAKE_CURRENT_SOURCE_DIR}/vendor/openssl")
set(_SFU_VENDOR_OPENSSL_WIN64 "${CMAKE_CURRENT_SOURCE_DIR}/vendor/OpenSSL-Win64")
if(WIN32 AND NOT OPENSSL_ROOT_DIR)
    if(EXISTS "${_SFU_VENDOR_OPENSSL}/include/openssl/opensslv.h")
        set(OPENSSL_ROOT_DIR "${_SFU_VENDOR_OPENSSL}" CACHE PATH
            "Prebuilt OpenSSL (local); see sfu/scripts/bootstrap-openssl.ps1" FORCE)
    elseif(EXISTS "${_SFU_VENDOR_OPENSSL_WIN64}/include/openssl/opensslv.h")
        set(OPENSSL_ROOT_DIR "${_SFU_VENDOR_OPENSSL_WIN64}" CACHE PATH
            "Prebuilt OpenSSL (OpenSSL-Win64 next to sfu/vendor)" FORCE)
    else()
        message(WARNING
            "OpenSSL dev tree not found under ${_SFU_VENDOR_OPENSSL} or ${_SFU_VENDOR_OPENSSL_WIN64}.\n"
            "libdatachannel (libsrtp) needs include/ + lib/ (e.g. lib/VC/x64/MD). Use Win64 *full*, not Light.\n"
            "Or set OPENSSL_ROOT_DIR, or run: pwsh -File ${CMAKE_CURRENT_SOURCE_DIR}/scripts/bootstrap-openssl.ps1")
    endif()
endif()

# -----------------------------------------------------------------------------
# libdatachannel (WebRTC) — recursive submodules under vendor/libdatachannel
# -----------------------------------------------------------------------------
set(LIBDATACHANNEL_DIR "${CMAKE_CURRENT_SOURCE_DIR}/vendor/libdatachannel")
if(NOT EXISTS "${LIBDATACHANNEL_DIR}/CMakeLists.txt")
    message(FATAL_ERROR
        "libdatachannel not found at ${LIBDATACHANNEL_DIR}.\n" 
        "From repository root run:\n"
        "  git submodule update --init --recursive sfu/vendor/libdatachannel\n")
endif()

if(NOT EXISTS "${LIBDATACHANNEL_DIR}/deps/json/CMakeLists.txt" OR
   NOT EXISTS "${LIBDATACHANNEL_DIR}/deps/plog/CMakeLists.txt" OR
   NOT EXISTS "${LIBDATACHANNEL_DIR}/deps/libjuice/CMakeLists.txt" OR
   NOT EXISTS "${LIBDATACHANNEL_DIR}/deps/libsrtp/CMakeLists.txt" OR
   NOT EXISTS "${LIBDATACHANNEL_DIR}/deps/usrsctp/CMakeLists.txt")
    message(FATAL_ERROR
        "libdatachannel dependencies missing (init its submodules).\n"
        "  cd ${LIBDATACHANNEL_DIR}\n"
        "  git submodule update --init --recursive\n")
endif()

# Static build (BUILD_SHARED_LIBS OFF): sample apps still run POST_BUILD copy of datachannel.dll on MSVC.
set(NO_EXAMPLES ON CACHE BOOL "Embedded libdatachannel: skip examples" FORCE)
set(NO_TESTS ON CACHE BOOL "Embedded libdatachannel: skip unit tests / benchmark" FORCE)

add_subdirectory("${LIBDATACHANNEL_DIR}" "${CMAKE_CURRENT_BINARY_DIR}/libdatachannel")

set(DATACHANNEL_TARGET datachannel)

# -----------------------------------------------------------------------------
# Executable
# -----------------------------------------------------------------------------
set(SFU_SOURCES
    "${CMAKE_CURRENT_SOURCE_DIR}/src/main.cpp"
    "${CMAKE_CURRENT_SOURCE_DIR}/src/sfu_service_impl.cpp"
)

add_executable(${PROJECT_NAME} ${SFU_SOURCES})
target_link_libraries(${PROJECT_NAME} PRIVATE
    sfu_grpc_proto
    ${DATACHANNEL_TARGET}
    absl::log_initialize
    absl::log_globals
)

target_include_directories(${PROJECT_NAME} PRIVATE
    "${CMAKE_CURRENT_SOURCE_DIR}/src"
    "${PROTO_GEN_DIR}"
    "${LIBDATACHANNEL_DIR}/include"
    "${LIBDATACHANNEL_DIR}/deps/json/include"
)

set_property(DIRECTORY "${CMAKE_CURRENT_SOURCE_DIR}" PROPERTY VS_STARTUP_PROJECT ${PROJECT_NAME})

if(MSVC)
    target_compile_options(${PROJECT_NAME} PRIVATE /utf-8 /W4)
else()
    target_compile_options(${PROJECT_NAME} PRIVATE -Wall -Wextra -Wpedantic)
endif()

if(WIN32)
    target_compile_definitions(${PROJECT_NAME} PRIVATE _WIN32_WINNT=0x0601)
endif()
