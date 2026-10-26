# QuicklIME TSF テキストサービス (in-proc COM DLL)
cmake_minimum_required(VERSION 3.21)
project(quicklime_tsf LANGUAGES CXX)

# CRT は静的リンク (/MT)。IME DLL は全プロセスにロードされるため、
# app-local な msvcp140.dll の配布では DLL 探索 (exe 基準) が効かない
set(CMAKE_MSVC_RUNTIME_LIBRARY "MultiThreaded$<$<CONFIG:Debug>:Debug>")

add_library(quicklime_tsf SHARED
    src/caret_marker.cpp
    src/config.cpp
    src/debug_log.cpp
    src/dll_main.cpp
    src/display_attribute.cpp
    src/edit_session.cpp
    src/engine_client.cpp
    src/globals.cpp
    src/icon_font.cpp
    src/kana_forms.cpp
    src/lang_bar.cpp
    src/mode_popup.cpp
    src/panel_theme.cpp
    src/preview.cpp
    src/registry.cpp
    src/romaji.cpp
    src/suggest_panel.cpp
    src/text_service.cpp
    src/text_service_direct.cpp
    src/exports.def
    src/QuicklIME.rc
)

# 出力ファイル名は QuicklIME.dll
set_target_properties(quicklime_tsf PROPERTIES OUTPUT_NAME "QuicklIME")

target_compile_features(quicklime_tsf PRIVATE cxx_std_20)
target_compile_definitions(quicklime_tsf PRIVATE UNICODE _UNICODE)
target_compile_options(quicklime_tsf PRIVATE /W4 /utf-8 /EHsc)
target_link_libraries(quicklime_tsf PRIVATE d2d1 dwrite dwmapi)

# AppContainer のアプリ (スタートメニューの検索欄など) が開発版の DLL を読み込めるようにする。
# インストール版は Program Files から同じ 2 つ (ALL APPLICATION PACKAGES と
# ALL RESTRICTED APPLICATION PACKAGES) の読み取りと実行を継承する
add_custom_command(TARGET quicklime_tsf POST_BUILD
    COMMAND icacls "$<SHELL_PATH:$<TARGET_FILE:quicklime_tsf>>" /grant "*S-1-15-2-1:RX" "*S-1-15-2-2:RX"
    VERBATIM)
