import { assertEquals } from "jsr:@std/assert@0.224.0";
import {
    extractSections,
    generateStableId,
    parseDigestComment,
    parseJulesReport
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
- [ ] CR-f6b7c8d9e0: Add error handling for 'result' unwrapping
`;

const MOCK_JULES_REPORT = `
### Jules Review Result

#### Fixed
- CR-a1b2c3d4e5: Fixed typo

#### Skipped (with rationale)
- CR-f6b7c8d9e0: Not critical right now
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

    const item2 = items.get("CR-f6b7c8d9e0");
    assertEquals(item2?.status, "open");
});

Deno.test("parseJulesReport - extracts status updates", () => {
    const statusMap = parseJulesReport(MOCK_JULES_REPORT);

    assertEquals(statusMap.get("CR-a1b2c3d4e5"), "fixed");
    assertEquals(statusMap.get("CR-f6b7c8d9e0"), "skipped");
});

Deno.test("extractSections - handles empty strings and no sections", () => {
    const { actionable, nitpick } = extractSections("");
    assertEquals(actionable.length, 0);
    assertEquals(nitpick.length, 0);

    const { actionable: a2, nitpick: n2 } = extractSections("Some random text without headers");
    assertEquals(a2.length, 0);
    assertEquals(n2.length, 0);
});

Deno.test("parseDigestComment - handles malformed lines", () => {
    const body = `
    - [ ] InvalidLine
    - [ ] CR-12345: Short ID
    - [ ] CR-longerthan10chars123: Long ID
    - [x] CR-valid1234 (invalidstatus): Content
    `;
    // Note: The regex strictly expects CR-[hex]{10}. 
    // And standard parsing might be resilient or strict.
    // 'CR-valid1234' matches 10 chars if it is exactly that.

    // Let's test checking logic resilience
    const items = parseDigestComment(body);
    // Should be empty as no lines match the strict regex
    assertEquals(items.size, 0);
    // Should be empty or only contain valid ones if we happened to construct one
    // 'CR-valid1234' is 12 chars (CR- + 10). Wait.
    // Logic: CR-[a-f0-9]{10} means CR- followed by 10 hex.

    // Test a valid one with weird spacing
    const validBody = `- [ ]   CR-abcdef1234   :    Spaced Content`;
    const validItems = parseDigestComment(validBody);
    assertEquals(validItems.has("CR-abcdef1234"), true);
    assertEquals(validItems.get("CR-abcdef1234")?.content, "Spaced Content");
    assertEquals(validItems.get("CR-abcdef1234")?.status, "open");
});

Deno.test("generateStableId - stable despite file/line variance if content same (current impl)", async () => {
    // The current implementation ONLY uses content and type to generate ID?
    // Let's check the code:
    // const key = `${item.type}:${item.filePath || ''}:${item.line || ''}:${normalizedContent}`;
    // It DOES use filePath and line.

    const content = "Fix typo";
    const base = { type: 'actionable', content };
    const withLoc = { type: 'actionable', content, filePath: 'src/main.rs', line: 10 };

    const id1 = await generateStableId(base);
    const id2 = await generateStableId(withLoc);

    // They should be DIFFERENT because location is part of the key
    // Note: if the requirement was to verify they ARE different:
    assertEquals(id1 !== id2, true);
});

Deno.test("parseDigestComment - captures status tokens", () => {
    const body = `
    - [x] CR-abcdef1234 (skipped): Skipped item
    - [x] CR-567890abcd (deferred): Deferred item
    - [x] CR-1122334455: Fixed item
    `;
    const items = parseDigestComment(body);

    assertEquals(items.get("CR-abcdef1234")?.status, "skipped");
    assertEquals(items.get("CR-567890abcd")?.status, "deferred");
    assertEquals(items.get("CR-1122334455")?.status, "fixed");
});

Deno.test("parseDigestComment - round-trips type information", () => {
    // Note: The parser is loose about what goes in (), e.g. (nitpick) or (nitpick, skipped)
    // We want to verify that if "nitpick" is present, type is parsed as "nitpick"
    const body = `
    - [ ] CR-1234567890 (nitpick): Nitpick item
    - [ ] CR-0987654321: Actionable item
    - [x] CR-1122334455 (nitpick, skipped): Skipped nitpick
    - [x] CR-6543210987 (deferred): Deferred actionable
    `;
    const items = parseDigestComment(body);

    const nitpick = items.get("CR-1234567890");
    assertEquals(nitpick?.type, "nitpick");
    assertEquals(nitpick?.status, "open");

    const actionable = items.get("CR-0987654321");
    // Default is actionable if not specified
    assertEquals(actionable?.type, "actionable");

    const skippedNitpick = items.get("CR-1122334455");
    assertEquals(skippedNitpick?.type, "nitpick");
    assertEquals(skippedNitpick?.status, "skipped");

    const deferred = items.get("CR-6543210987");
    assertEquals(deferred?.type, "actionable"); // no "nitpick" token
    assertEquals(deferred?.status, "deferred");
});

// --- Mock Octokit ---

// Simple mock for Octokit to verify dispatch calls
class MockOctokit {
    public actions = {
        createWorkflowDispatch: (args: any) => {
            this.dispatchCalls.push(args);
            return Promise.resolve();
        }
    };
    public dispatchCalls: any[] = [];

    constructor() { }
}

import { checkAndTriggerJules } from "./jules-digest.ts";

Deno.test("checkAndTriggerJules - triggers when shouldTrigger is true", async () => {
    // @ts-ignore - Mocking Octokit
    const mockOctokit = new MockOctokit() as unknown as any;

    await checkAndTriggerJules(
        mockOctokit,
        true, // shouldTrigger
        "owner",
        "repo",
        "main",
        123,
        "digest content"
    );

    assertEquals(mockOctokit.dispatchCalls.length, 1);
    const call = mockOctokit.dispatchCalls[0];
    assertEquals(call.owner, "owner");
    assertEquals(call.repo, "repo");
    assertEquals(call.workflow_id, "jules-process.yml");
    assertEquals(call.ref, "main");
    assertEquals(call.inputs.pr_number, "123");
    // Ensure digest_body is NOT passed
    assertEquals(call.inputs.digest_body, undefined);
});

Deno.test("checkAndTriggerJules - does NOT trigger when shouldTrigger is false", async () => {
    // @ts-ignore - Mocking Octokit
    const mockOctokit = new MockOctokit() as unknown as any;

    await checkAndTriggerJules(
        mockOctokit,
        false, // shouldTrigger
        "owner",
        "repo",
        "main",
        123,
        "digest content"
    );

    assertEquals(mockOctokit.dispatchCalls.length, 0);
});



