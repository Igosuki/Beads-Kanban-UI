"use client";

import { cn } from "@/lib/utils";
import type { Bead, BeadStatus, PRChecks } from "@/types";
import { Check, Circle, Clock, FileCheck, GitPullRequest, GitMerge } from "lucide-react";

/**
 * PR status for a child task (used for icon display)
 */
export interface ChildPRStatus {
  state: "open" | "merged" | "closed";
  checks: { status: "success" | "failure" | "pending" };
}

export interface SubtaskListProps {
  /** Child tasks to display */
  childTasks: Bead[];
  /** Callback when clicking a child task */
  onChildClick: (child: Bead) => void;
  /** Maximum number of children to show when collapsed */
  maxCollapsed?: number;
  /** Whether the list is expanded */
  isExpanded?: boolean;
  /** PR status for each child task, keyed by bead ID */
  childPRStatuses?: Map<string, ChildPRStatus>;
}

/**
 * Get status icon based on bead status
 */
function getStatusIcon(status: BeadStatus) {
  switch (status) {
    case 'closed':
      return <Check className="h-3.5 w-3.5 text-green-400" aria-hidden="true" />;
    case 'in_progress':
      return <Clock className="h-3.5 w-3.5 text-blue-400" aria-hidden="true" />;
    case 'inreview':
      return <FileCheck className="h-3.5 w-3.5 text-purple-400" aria-hidden="true" />;
    case 'open':
    default:
      return <Circle className="h-3.5 w-3.5 text-zinc-500" aria-hidden="true" />;
  }
}

/**
 * Get status text color
 */
function getStatusColor(status: BeadStatus): string {
  switch (status) {
    case 'closed':
      return "text-green-400";
    case 'in_progress':
      return "text-blue-400";
    case 'inreview':
      return "text-purple-400";
    case 'open':
    default:
      return "text-zinc-500";
  }
}

/**
 * Get PR status icon based on PR state and checks
 * Returns null if no PR status (no icon shown)
 */
function getPRStatusIcon(prStatus: ChildPRStatus | undefined): React.ReactNode {
  if (!prStatus) {
    // No PR - no icon
    return null;
  }

  if (prStatus.state === "merged") {
    // Merged PR - purple GitMerge icon
    return (
      <GitMerge
        className="h-3.5 w-3.5 text-purple-400"
        aria-label="PR merged"
      />
    );
  }

  if (prStatus.state === "open") {
    // Open PR - color based on checks status
    if (prStatus.checks.status === "success") {
      return (
        <GitPullRequest
          className="h-3.5 w-3.5 text-green-400"
          aria-label="PR open, checks passing"
        />
      );
    }
    if (prStatus.checks.status === "failure") {
      return (
        <GitPullRequest
          className="h-3.5 w-3.5 text-red-400"
          aria-label="PR open, checks failing"
        />
      );
    }
    // Pending checks
    return (
      <GitPullRequest
        className="h-3.5 w-3.5 text-amber-400"
        aria-label="PR open, checks pending"
      />
    );
  }

  // Closed PR (not merged) - no icon
  return null;
}

/**
 * Truncate text to max length
 */
function truncate(text: string, maxLength: number): string {
  if (text.length <= maxLength) return text;
  return text.slice(0, maxLength).trim() + "…";
}

/**
 * Compact list of child tasks within epic card
 */
export function SubtaskList({
  childTasks,
  onChildClick,
  maxCollapsed = 3,
  isExpanded = false,
  childPRStatuses,
}: SubtaskListProps) {
  if (childTasks.length === 0) {
    return (
      <div className="text-xs text-muted-foreground italic">
        No child tasks
      </div>
    );
  }

  const displayChildren = isExpanded ? childTasks : childTasks.slice(0, maxCollapsed);
  const hasMore = childTasks.length > maxCollapsed && !isExpanded;

  return (
    <div className="space-y-1">
      {displayChildren.map((child) => (
        <button
          key={child.id}
          onClick={(e) => {
            e.stopPropagation();
            onChildClick(child);
          }}
          aria-label={`Open task: ${child.title}`}
          className={cn(
            "w-full flex items-start gap-2 px-2 py-1.5 rounded-md",
            "hover:bg-zinc-800 transition-colors text-left",
            "focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-zinc-400",
            "group"
          )}
        >
          <div className="flex items-center gap-1 flex-shrink-0 mt-0.5">
            {getStatusIcon(child.status)}
            {getPRStatusIcon(childPRStatuses?.get(child.id))}
          </div>
          <div className="flex-1 min-w-0">
            <p className={cn(
              "text-xs font-medium group-hover:underline",
              child.status === 'closed' && "line-through text-zinc-500",
              child.status !== 'closed' && "text-zinc-200"
            )}>
              {truncate(child.title, 50)}
            </p>
            {child.description && (
              <p className="text-[10px] text-zinc-500 mt-0.5">
                {truncate(child.description, 60)}
              </p>
            )}
          </div>
          <div className={cn(
            "flex-shrink-0 text-[9px] font-medium uppercase tracking-wide",
            getStatusColor(child.status)
          )}>
            {child.status.replace('_', ' ')}
          </div>
        </button>
      ))}
      {hasMore && (
        <p className="text-[10px] text-muted-foreground text-center py-1">
          +{childTasks.length - maxCollapsed} more
        </p>
      )}
    </div>
  );
}
