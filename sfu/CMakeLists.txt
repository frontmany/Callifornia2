cmake_minimum_required(VERSION 3.16)
project(sfu VERSION 1.0.0 LANGUAGES C CXX)

set(CMAKE_CXX_STANDARD 20)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_CXX_EXTENSIONS OFF)

option(BUILD_SHARED_LIBS "Build shared libraries" OFF)

set(LIBDATACHANNEL_DIR ${CMAKE_CURRENT_SOURCE_DIR}/vendor/libdatachannel)

if(NOT EXISTS ${LIBDATACHANNEL_DIR}/CMakeLists.txt)
    message(FATAL_ERROR
        "libdatachannel not found in ${LIBDATACHANNEL_DIR}\n"
        "Please run:\n"
        "  git submodule add https://github.com/paullouisageneau/libdatachannel.git vendor/libdatachannel\n"
        "  cd ${LIBDATACHANNEL_DIR}\n"
        "  git submodule update --init --recursive --depth 1\n"
    )
endif()

if(NOT EXISTS ${LIBDATACHANNEL_DIR}/deps/json/CMakeLists.txt OR
   NOT EXISTS ${LIBDATACHANNEL_DIR}/deps/plog/CMakeLists.txt OR
   NOT EXISTS ${LIBDATACHANNEL_DIR}/deps/libjuice/CMakeLists.txt OR
   NOT EXISTS ${LIBDATACHANNEL_DIR}/deps/libsrtp/CMakeLists.txt OR
   NOT EXISTS ${LIBDATACHANNEL_DIR}/deps/usrsctp/CMakeLists.txt)
    message(FATAL_ERROR
        "libdatachannel dependencies are missing!\n"
        "Please initialize all submodules:\n"
        "  cd ${LIBDATACHANNEL_DIR}\n"
        "  git submodule update --init --recursive --depth 1\n"
    )
endif()

add_subdirectory(${LIBDATACHANNEL_DIR} ${CMAKE_CURRENT_BINARY_DIR}/libdatachannel)

set(DATACHANNEL_TARGET libdatachannel)

file(GLOB_RECURSE SOURCES "src/*.cpp" "src/*.h")
source_group("Source Files" FILES ${SOURCES})

add_executable(${PROJECT_NAME} ${SOURCES})

target_link_libraries(${PROJECT_NAME} PRIVATE ${DATACHANNEL_TARGET})

target_include_directories(${PROJECT_NAME} PRIVATE
    ${CMAKE_CURRENT_SOURCE_DIR}/src
    ${LIBDATACHANNEL_DIR}/include
    ${LIBDATACHANNEL_DIR}/deps/json/include
)

if(MSVC)
    target_compile_options(${PROJECT_NAME} PRIVATE /utf-8 /W4)
else()
    target_compile_options(${PROJECT_NAME} PRIVATE -Wall -Wextra -Wpedantic)
endif()

if(WIN32)
    target_compile_definitions(${PROJECT_NAME} PRIVATE _WIN32_WINNT=0x0601)
endif()