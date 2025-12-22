#!/bin/sh

ast-grep scan --rule "$(temp=$( realpath "$0" ) && dirname "$temp")/find_unsafe_statics.yml" $(fd -e rs -E examples -E tests -E benches -E "objc2*" -E "windows*" -E "ndk*" -E "harfrust*" -E "core-graphics*" -E "syn*" -E "thiserror*" -E "rav1e*")
