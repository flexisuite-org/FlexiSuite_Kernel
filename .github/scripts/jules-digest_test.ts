import { assertEquals } from "jsr:@std/assert@0.224.0";
import {
    extractSections,
    generateStableId,
    parseDigestComment,
    parseJulesReport,
    ReviewItem
} from "./jules-digest.ts";

// --- Mock Data ---

const MOCK_CODERABBIT_REVIEW = `
## Walkthrough
The changes involve...

## Changes
...

## Actionable
- [ ] Fix the typo in connection.rs
- [ ] Add error handling for 'result' unwrapping

## Nitpicks
- Use const instead of let for 'url'
- Add a trailing comma
`;

const MOCK_DIGEST_BODY = `
## CodeRabbit Digest for Jules

**Actionable & Nitpicks**
- [x] CR-a1b2c3d4e5: Fix the typo in connection.rs
- [ ] CR-f6g7h8i9j0: Add error handling for 'result' unwrapping
`;

const MOCK_JULES_REPORT = `
### Jules Review Result

#### Fixed
- CR-a1b2c3d4e5: Fixed typo

#### Skipped (with rationale)
- CR-f6g7h8i9j0: Not critical right now
`;

// --- Tests ---

Deno.test("extractSections - parse basic CodeRabbit review", () => {
    const { actionable, nitpick } = extractSections(MOCK_CODERABBIT_REVIEW);

    assertEquals(actionable.length, 2);
    assertEquals(actionable[0], "Fix the typo in connection.rs");

    assertEquals(nitpick.length, 2);
    assertEquals(nitpick[0], "Use const instead of let for 'url'");
});

Deno.test("generateStableId - generates consistent ID", async () => {
    const item1 = { type: 'actionable', content: "Fix typo" };
    const id1 = await generateStableId(item1);

    const item2 = { type: 'actionable', content: "  Fix   typo  " }; // Whitespace diff
    const id2 = await generateStableId(item2);

    assertEquals(id1, id2);
    assertEquals(id1.startsWith("CR-"), true);
    assertEquals(id1.length, 13); // CR- + 10 chars
});

Deno.test("parseDigestComment - extracts existing items and status", () => {
    const items = parseDigestComment(MOCK_DIGEST_BODY);

    assertEquals(items.size, 2);

    const item1 = items.get("CR-a1b2c3d4e5");
    assertEquals(item1?.status, "fixed");

    const item2 = items.get("CR-f6g7h8i9j0");
    assertEquals(item2?.status, "open");
});

Deno.test("parseJulesReport - extracts status updates", () => {
    const statusMap = parseJulesReport(MOCK_JULES_REPORT);

    assertEquals(statusMap.get("CR-a1b2c3d4e5"), "fixed");
    assertEquals(statusMap.get("CR-f6g7h8i9j0"), "skipped");
});
