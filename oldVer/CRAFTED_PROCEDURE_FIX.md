# Crafted Procedure Signature Fix

## v3.4.0 Beta 3 hotfix

Fixed a generated-item compilation mismatch where an item class could call:

```java
SomeItemIsCraftedProcedure.execute();
```

while the generated MCreator procedure correctly requires:

```java
SomeItemIsCraftedProcedure.execute(world, itemstack);
```

The source bridge now:

- emits the context-aware `onCraftedBy` hook for newly generated item/tool classes;
- repairs stale zero-argument calls in existing generated item classes during the compile bridge pass;
- leaves unrelated procedure calls untouched;
- reports the number of repaired crafted-procedure calls in `MOJMODELBRIDGE_SOURCE_BRIDGE_REPORT.json`.

This matches the working 1.21.1 NeoForge example's generated item behavior.
