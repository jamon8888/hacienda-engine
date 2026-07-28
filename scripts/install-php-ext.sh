#!/bin/bash
set -e


EXTENSION_DIR=$(php -r 'echo ini_get("extension_dir");')

for path in target/release/libxberg_php.dylib target/release/libxberg_php.so target/release/xberg_php.dll; do
  if [ -f "$path" ]; then
    EXT_PATH="$path"
    break
  fi
done

if [ -z "$EXT_PATH" ]; then
  echo "Error: PHP extension not found in target/release/" >&2
  exit 1
fi

EXT_FILENAME=$(basename "$EXT_PATH")
cp "$EXT_PATH" "$EXTENSION_DIR/$EXT_FILENAME"

PHP_INI=$(php -r 'echo php_ini_loaded_file();')
if ! grep -q "extension=$EXT_FILENAME" "$PHP_INI"; then
  echo "extension=$EXT_FILENAME" >>"$PHP_INI"
fi

echo "Installed PHP extension: $EXT_FILENAME to $EXTENSION_DIR"
