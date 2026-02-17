import { Octokit } from "npm:@octokit/rest@20.0.2";
import { crypto } from "jsr:@std/crypto@0.224.0"; // Using standard library for hashing

// --- Type Definitions ---

export interface ReviewItem {
    id: string;
    type: 'actionable' | 'nitpick';
    content: string; // Markdown content, preserved from CodeRabbit
    status: 'open' | 'fixed' | 'skipped' | 'deferred';
    url?: string;
    filePath?: string;
    line?: number;
}

interface DigestContext {
    digestCommentId?: number;
    items: Map<string, ReviewItem>;
}

// --- Configuration ---

const DIGEST_HEADER = "## CodeRabbit Digest for Jules";
const JULES_REPORT_HEADER = "### Jules Review Result";
const UNRESOLVED_HEADER = "## CodeRabbit Digest for Jules (Unresolved Items)";

// --- Helper Functions ---

async function generateStableId(item: { type: string, content: string, filePath?: string, line?: number }): Promise<string> {
    // Normalize content: trim and collapse whitespace
    const normalizedContent = item.content.trim().replace(/\s+/g, ' ');
    const key = `${item.type}:${item.filePath || ''}:${item.line || ''}:${normalizedContent}`;

    const encoder = new TextEncoder();
    const data = encoder.encode(key);
    const hashBuffer = await crypto.subtle.digest("SHA-1", data);
    const hashArray = Array.from(new Uint8Array(hashBuffer));
    const hashHex = hashArray.map(b => b.toString(16).padStart(2, '0')).join('');

    return `CR-${hashHex.substring(0, 10)}`;
}

// --- Parsing Logic ---

function extractSections(body: string): { actionable: string[], nitpick: string[] } {
    // Simple regex-based extraction. 
    // CodeRabbit format varies, but usually has "## Actionable..." and "## Nitpicks..."
    // This is a simplified implementation. Real-world parsing might need to be more robust.

    const actionableItems: string[] = [];
    const nitpickItems: string[] = [];

    // Split by potential headers
    const lines = body.split('\n');
    let currentSection: 'none' | 'actionable' | 'nitpick' = 'none';

    for (const line of lines) {
        if (line.match(/##.*Actionable/i) || line.match(/Fix all issues/i)) {
            currentSection = 'actionable';
            continue;
        } else if (line.match(/##.*Nitpick/i)) {
            currentSection = 'nitpick';
            continue;
        } else if (line.startsWith('## ')) {
            currentSection = 'none';
        }

        if (currentSection !== 'none' && (line.trim().startsWith('- ') || line.trim().startsWith('* '))) {
            const content = line.trim().substring(2).trim(); // Remove bullet point
            if (currentSection === 'actionable') {
                actionableItems.push(content);
            } else {
                nitpickItems.push(content);
            }
        }
    }

    return { actionable: actionableItems, nitpick: nitpickItems };
}

function parseDigestComment(body: string): Map<string, ReviewItem> {
    const items = new Map<string, ReviewItem>();
    const lines = body.split('\n');

    for (const line of lines) {
        // Regex to capture: - [x] CR-abcdef1234: Content...
        const match = line.match(/^\s*-\s*\[([ xX])\]\s*(CR-[a-f0-9]{10}):\s*(.*)$/);
        if (match) {
            const isChecked = match[1].toLowerCase() === 'x';
            const id = match[2];
            const content = match[3];

            items.set(id, {
                id,
                type: 'actionable', // We lose type info in serialized form, assume actionable or maintain via context? 
                // Actually, type isn't strictly needed for status tracking.
                content,
                status: isChecked ? 'fixed' : 'open',
            });
        }
    }
    return items;
}


function parseJulesReport(body: string): Map<string, 'fixed' | 'skipped' | 'deferred'> {
    const statusMap = new Map<string, 'fixed' | 'skipped' | 'deferred'>();
    const lines = body.split('\n');
    let currentStatus: 'fixed' | 'skipped' | 'deferred' | null = null;

    for (const line of lines) {
        if (line.match(/####.*Fixed/i)) currentStatus = 'fixed';
        else if (line.match(/####.*Skipped/i)) currentStatus = 'skipped';
        else if (line.match(/####.*Deferred/i)) currentStatus = 'deferred';
        else if (line.startsWith('#')) currentStatus = null;

        if (currentStatus) {
            const match = line.match(/(CR-[a-f0-9]{10})/);
            if (match) {
                statusMap.set(match[1], currentStatus);
            }
        }
    }
    return statusMap;
}

// --- Verification/Sweep Logic ---

async function run() {
    const token = Deno.env.get("GITHUB_TOKEN");
    if (!token) {
        console.error("GITHUB_TOKEN is missing");
        Deno.exit(1);
    }
    const octokit = new Octokit({ auth: token });

    const eventPath = Deno.env.get("GITHUB_EVENT_PATH");
    const eventName = Deno.env.get("GITHUB_EVENT_NAME");

    if (!eventPath || !eventName) {
        console.error("Missing event path or name");
        Deno.exit(1);
    }

    const payload = JSON.parse(await Deno.readTextFile(eventPath));

    let prNumber: number | undefined;
    let repoOwner: string | undefined;
    let repoName: string | undefined;

    // Extract PR context
    if (payload.pull_request) {
        prNumber = payload.pull_request.number;
        repoOwner = payload.repository.owner.login;
        repoName = payload.repository.name;
    } else if (payload.issue && payload.issue.pull_request) {
        prNumber = payload.issue.number;
        repoOwner = payload.repository.owner.login;
        repoName = payload.repository.name;
    }

    if (!prNumber || !repoOwner || !repoName) {
        console.log("Not a PR event, skipping.");
        return;
    }

    console.log(`Processing PR #${prNumber} in ${repoOwner}/${repoName}`);

    // Fetch PR comments to find Digest and Jules Report
    const comments = await octokit.paginate(octokit.issues.listComments, {
        owner: repoOwner,
        repo: repoName,
        issue_number: prNumber,
    });

    let digestComment = comments.find(c => c.body && (c.body.includes(DIGEST_HEADER) || c.body.includes(UNRESOLVED_HEADER)));
    let existingItems = new Map<string, ReviewItem>();

    if (digestComment && digestComment.body) {
        existingItems = parseDigestComment(digestComment.body);
    }

    // Parse Jules Reports to update status
    const julesReports = comments.filter(c => c.body && c.body.includes(JULES_REPORT_HEADER));
    for (const report of julesReports) {
        if (!report.body) continue;
        const statuses = parseJulesReport(report.body);
        for (const [id, status] of statuses) {
            if (existingItems.has(id)) {
                const item = existingItems.get(id)!;
                // Report status overrides check status unless it's 'open'
                item.status = status;
            }
        }
    }

    // If this event is a CodeRabbit review, extract new items
    const newItems: ReviewItem[] = [];

    // Logic to fetch CodeRabbit comments/reviews if this is a review event
    // For simplicity, we assume we fetch the latest review from CodeRabbit or process the event body if it's a review submission.
    // Ideally, query recent reviews by bot user 'coderabbitai'.

    const reviews = await octokit.pulls.listReviews({
        owner: repoOwner,
        repo: repoName,
        pull_number: prNumber,
    });

    const codeRabbitReviews = reviews.data.filter(r => r.user?.login === 'coderabbitai' || r.user?.login?.includes('coderabbit')); // Adjust bot name

    // Process the latest review body
    if (codeRabbitReviews.length > 0) {
        // Get the latest one or all of them? 
        // Strategy: Collect ALL items from ALL valid CodeRabbit reviews to ensure we have a complete picture, 
        // or just the latest?
        // Requirement says "Accumulate". Let's process the latest one for now, or maybe loop all.
        // Better to process the latest *submitted* review content.

        const latestReview = codeRabbitReviews[codeRabbitReviews.length - 1]; // Simply last
        if (latestReview.body) {
            const extracted = extractSections(latestReview.body);

            for (const content of extracted.actionable) {
                const id = await generateStableId({ type: 'actionable', content });
                newItems.push({ id, type: 'actionable', content, status: 'open' });
            }
            for (const content of extracted.nitpick) {
                const id = await generateStableId({ type: 'nitpick', content });
                newItems.push({ id, type: 'nitpick', content, status: 'open' });
            }
        }
    }

    // Reconcile
    for (const item of newItems) {
        if (!existingItems.has(item.id)) {
            existingItems.set(item.id, item);
        }
        // If it exists, we keep the status (managed by check/report)
    }

    // Generate New Digest Body
    const sortedItems = Array.from(existingItems.values()).sort((a, b) => {
        // Sort open items first
        if (a.status === 'open' && b.status !== 'open') return -1;
        if (a.status !== 'open' && b.status === 'open') return 1;
        return 0;
    });

    // Check list format
    let newBody = `${DIGEST_HEADER}\n\n`;
    newBody += `**Actionable & Nitpicks**\n`;

    let hasOpenItems = false;

    for (const item of sortedItems) {
        const checked = (item.status === 'fixed' || item.status === 'skipped' || item.status === 'deferred') ? 'x' : ' ';
        newBody += `- [${checked}] ${item.id}: ${item.content}\n`; // Link would be ideal here
        if (item.status === 'open') hasOpenItems = true;
    }

    // Sweep logic: If we are in a sweep mode (e.g. specialized trigger) or just always hinting
    if (hasOpenItems) {
        newBody = `@Jules\n` + newBody.replace(DIGEST_HEADER, UNRESOLVED_HEADER);
        newBody += `\n\nThere are unresolved items. Please review and update status (Fix/Skip/Defer) for the unchecked items.`;
    } else {
        newBody += `\n\nAll items resolved! code review complete.`;
    }

    // Post/Update
    if (digestComment) {
        // Don't update if content is identical to avoid spam, unless we need to poke (Sweep)
        // Check if body essentially changed (ignoring timestamp or dynamic footer)
        if (digestComment.body?.trim() !== newBody.trim()) {
            await octokit.issues.updateComment({
                owner: repoOwner,
                repo: repoName,
                comment_id: digestComment.id,
                body: newBody
            });
            console.log("Updated Digest comment.");
        } else {
            console.log("Digest up to date.");
        }
    } else {
        if (sortedItems.length > 0) {
            await octokit.issues.createComment({
                owner: repoOwner,
                repo: repoName,
                issue_number: prNumber,
                body: newBody
            });
            console.log("Created Digest comment.");
        }
    }
}

// Run the script
// --- Exports for Testing ---

export {
    extractSections,
    generateStableId,
    parseDigestComment,
    parseJulesReport
};

// Run the script
if (import.meta.main) {
    try {
        await run();
    } catch (error) {
        console.error("Error running script:", error);
        Deno.exit(1);
    }
}
