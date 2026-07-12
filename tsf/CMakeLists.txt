# QuicklIME TSF テキストサービス (in-proc COM DLL)
cmake_minimum_required(VERSION 3.21)
project(quicklime_tsf LANGUAGES CXX)

add_library(quicklime_tsf SHARED
    src/dll_main.cpp
    src/globals.cpp
    src/registry.cpp
    src/romaji.cpp
    src/text_service.cpp
    src/exports.def
)

# 出力ファイル名は QuicklIME.dll
set_target_properties(quicklime_tsf PROPERTIES OUTPUT_NAME "QuicklIME")

target_compile_features(quicklime_tsf PRIVATE cxx_std_20)
target_compile_definitions(quicklime_tsf PRIVATE UNICODE _UNICODE)
target_compile_options(quicklime_tsf PRIVATE /W4 /utf-8 /EHsc)
