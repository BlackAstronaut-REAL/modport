# Crafted Procedure Dependency Fix

The previous fix attempted to patch generated Java calls. That was the wrong layer: MCreator regenerates those calls from the procedure's dependency metadata.

The supplied working NeoForge 1.21.1 workspace shows generated procedures carrying:

- `itemstack` dependency (`type: itemstack`)
- `world` dependency (`type: world`)

The converter now emits the same workspace metadata for every source-derived crafted-enchantment procedure. This makes MCreator generate:

```java
Procedure.execute(world, itemstack);
```

instead of:

```java
Procedure.execute();
```

This is generator-native and avoids post-generation Java surgery.
