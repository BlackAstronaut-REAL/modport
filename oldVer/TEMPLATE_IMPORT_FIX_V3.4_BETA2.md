# Template import fix

This pass fixes an invalid visual-field reset introduced by the template import cleanup.

For newly imported elements, the importer now resets template-bound model fields to safe
native defaults instead of leaving a custom render type pointing at an empty/deleted model.
It also repopulates required texture/gui/particle/face fields from the imported source asset.

Existing template elements are not passed through the cleanup path.

The suite passes 59/59 tests.
