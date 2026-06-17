
$ErrorActionPreference = "Stop"
cargo build
echo "Formatting volume with AEAD..."
$Env:RUST_BACKTRACE=1
echo "password`npassword" | .\target\debug\cfs-io.exe format test_vol.img 2M --encrypted --aead --kdf pbkdf2 --pbkdf2-iters 100000
echo "Testing cfs slot --list..."
echo "password" | .\target\debug\cfs-io.exe slot test_vol.img --list
echo "Testing cfs slot --add..."
echo "password`nnewpass`nnewpass" | .\target\debug\cfs-io.exe slot test_vol.img --add
echo "Testing cfs slot --list after add..."
echo "password" | .\target\debug\cfs-io.exe slot test_vol.img --list
echo "Testing cfs slot --remove 1..."
echo "password" | .\target\debug\cfs-io.exe slot test_vol.img --remove 1
echo "Testing cfs slot --list after remove..."
echo "password" | .\target\debug\cfs-io.exe slot test_vol.img --list

