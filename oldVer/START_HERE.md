# Start Here — MojModelBridge 3.4.0 Beta 2

1. Read `README.md` for capabilities and limitations.
2. When you have separate compiled and decompiled folders, use `full_port_combined.bat`.
3. When everything is already under one mod root, use `full_port.bat`.
4. Read `TESTING_GUIDE_V3.4_BETA2.md` before reporting runtime issues.
5. Keep the generated HTML/JSON reports and relevant logs with each test result.

Quick combined example:

```bat
full_port_combined.bat "D:\Mod\Compiled" "D:\Mod\Decompiled" "D:\Converted\Mod"
```

Quick one-root example:

```bat
full_port.bat "D:\Mod" "D:\Converted\Mod"
```
