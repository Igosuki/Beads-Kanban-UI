"use client";

import { useState, useCallback } from "react";
import {
  Sheet,
  SheetContent,
  SheetHeader,
  SheetTitle,
  SheetDescription,
} from "@/components/ui/sheet";
import { ScrollArea } from "@/components/ui/scroll-area";
import { cn } from "@/lib/utils";
import { Bot, ChevronDown, Loader2, Wrench } from "lucide-react";
import { useAgents } from "@/hooks/use-agents";
import type { Agent, AgentModel } from "@/types";

export interface AgentsPanelProps {
  /** Whether the panel is open */
  open: boolean;
  /** Callback when open state changes */
  onOpenChange: (open: boolean) => void;
  /** Absolute path to the project root */
  projectPath: string;
}

/**
 * Model badge color configuration.
 * Each model gets a distinct color for quick visual identification.
 */
const MODEL_COLORS: Record<
  AgentModel,
  { bg: string; text: string; border: string }
> = {
  opus: {
    bg: "bg-orange-500/15",
    text: "text-orange-400",
    border: "border-orange-500/25",
  },
  sonnet: {
    bg: "bg-cyan-500/15",
    text: "text-cyan-400",
    border: "border-cyan-500/25",
  },
  haiku: {
    bg: "bg-lime-500/15",
    text: "text-lime-400",
    border: "border-lime-500/25",
  },
};

/** All available model options */
const MODEL_OPTIONS: AgentModel[] = ["haiku", "sonnet", "opus"];

/**
 * Format the tools display for collapsed view.
 */
function formatToolsSummary(tools: string[] | "*"): string {
  if (tools === "*") return "All tools";
  if (tools.length === 0) return "No tools";
  if (tools.length <= 2) return tools.join(", ");
  return `${tools.slice(0, 2).join(", ")} +${tools.length - 2}`;
}

/**
 * Model badge component for displaying the current model.
 */
function ModelBadge({ model }: { model: AgentModel }) {
  const colors = MODEL_COLORS[model];
  return (
    <span
      className={cn(
        "inline-flex items-center rounded-sm px-1.5 py-0.5 text-[0.625rem] leading-none font-medium border",
        colors.bg,
        colors.text,
        colors.border
      )}
    >
      {model}
    </span>
  );
}

/**
 * Single agent card with collapsible details.
 */
function AgentCard({
  agent,
  isExpanded,
  onToggle,
  onUpdateModel,
  onToggleAllTools,
  isUpdating,
}: {
  agent: Agent;
  isExpanded: boolean;
  onToggle: () => void;
  onUpdateModel: (model: AgentModel) => void;
  onToggleAllTools: () => void;
  isUpdating: boolean;
}) {
  const hasAllTools = agent.tools === "*";

  return (
    <div className="rounded-lg border border-zinc-800 bg-zinc-900/50 overflow-hidden">
      {/* Collapsed header - always visible, clickable */}
      <button
        type="button"
        onClick={onToggle}
        aria-expanded={isExpanded}
        className="w-full text-left p-3 flex items-start gap-2.5 hover:bg-zinc-800/30 transition-colors focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring focus-visible:ring-inset"
      >
        <div className="flex-1 min-w-0 space-y-1">
          {/* Name row */}
          <div className="flex items-center gap-2">
            <span className="text-sm font-medium text-zinc-100 truncate">
              {agent.name}
            </span>
            <ModelBadge model={agent.model} />
          </div>

          {/* Description */}
          <p className="text-xs text-zinc-500 line-clamp-1 text-pretty">
            {agent.description || "No description"}
          </p>

          {/* Tools summary */}
          <div className="flex items-center gap-1 text-xs text-zinc-500">
            <Wrench className="size-3 shrink-0" aria-hidden="true" />
            <span className="truncate">{formatToolsSummary(agent.tools)}</span>
          </div>
        </div>

        {/* Expand chevron */}
        <ChevronDown
          className={cn(
            "size-4 text-zinc-500 shrink-0 mt-0.5 transition-transform",
            isExpanded && "rotate-180"
          )}
          aria-hidden="true"
        />
      </button>

      {/* Expanded details */}
      {isExpanded && (
        <div className="border-t border-zinc-800 p-3 space-y-3">
          {/* Model selector */}
          <div className="space-y-1.5">
            <span id={`model-label-${agent.filename}`} className="text-xs font-medium text-zinc-400">Model</span>
            <div
              className="flex gap-1"
              role="radiogroup"
              aria-labelledby={`model-label-${agent.filename}`}
            >
              {MODEL_OPTIONS.map((model) => {
                const isSelected = agent.model === model;
                const colors = MODEL_COLORS[model];
                return (
                  <button
                    key={model}
                    type="button"
                    role="radio"
                    aria-checked={isSelected}
                    disabled={isUpdating}
                    onClick={() => onUpdateModel(model)}
                    className={cn(
                      "flex-1 h-7 rounded-md text-xs font-medium transition-colors focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring",
                      isSelected
                        ? cn(colors.bg, colors.text, "border", colors.border)
                        : "bg-zinc-800/50 text-zinc-500 hover:text-zinc-300 hover:bg-zinc-800 border border-transparent"
                    )}
                  >
                    {model}
                  </button>
                );
              })}
            </div>
          </div>

          {/* All-tools toggle */}
          <div className="flex items-center justify-between">
            <label
              htmlFor={`all-tools-${agent.filename}`}
              className="text-xs font-medium text-zinc-400"
            >
              All tools
            </label>
            <button
              id={`all-tools-${agent.filename}`}
              type="button"
              role="switch"
              aria-checked={hasAllTools}
              disabled={isUpdating}
              onClick={onToggleAllTools}
              className={cn(
                "relative inline-flex h-5 w-9 shrink-0 items-center rounded-full border transition-colors focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring",
                hasAllTools
                  ? "bg-orange-500/30 border-orange-500/40"
                  : "bg-zinc-800 border-zinc-700"
              )}
            >
              <span
                className={cn(
                  "pointer-events-none block size-3.5 rounded-full transition-transform",
                  hasAllTools
                    ? "translate-x-[18px] bg-orange-400"
                    : "translate-x-[3px] bg-zinc-500"
                )}
              />
            </button>
          </div>

          {/* Tools list (when not all tools) */}
          {!hasAllTools && Array.isArray(agent.tools) && agent.tools.length > 0 && (
            <div className="space-y-1.5">
              <p className="text-xs font-medium text-zinc-400">
                Tools ({agent.tools.length})
              </p>
              <div className="flex flex-wrap gap-1">
                {agent.tools.map((tool) => (
                  <span
                    key={tool}
                    className="inline-flex items-center rounded-sm px-1.5 py-0.5 text-[0.625rem] leading-none font-mono bg-zinc-800 text-zinc-500 border border-zinc-700/50"
                  >
                    {tool}
                  </span>
                ))}
              </div>
            </div>
          )}

          {/* Updating indicator */}
          {isUpdating && (
            <div role="status" aria-live="polite" className="flex items-center gap-1.5 text-xs text-zinc-500">
              <Loader2
                className="size-3 animate-spin"
                aria-hidden="true"
              />
              Saving...
            </div>
          )}
        </div>
      )}
    </div>
  );
}

/**
 * Agents Panel - slide-out Sheet for browsing and configuring project agents
 */
export function AgentsPanel({
  open,
  onOpenChange,
  projectPath,
}: AgentsPanelProps) {
  const { agents, isLoading, error, updateAgent } = useAgents(projectPath);

  // Track which card is expanded
  const [expandedFilename, setExpandedFilename] = useState<string | null>(null);

  // Track which agent is being updated
  const [updatingFilename, setUpdatingFilename] = useState<string | null>(null);

  /**
   * Toggle expanded state for a card
   */
  const handleToggle = useCallback((filename: string) => {
    setExpandedFilename((prev) => (prev === filename ? null : filename));
  }, []);

  /**
   * Handle model change for an agent
   */
  const handleUpdateModel = useCallback(
    async (agent: Agent, model: AgentModel) => {
      if (model === agent.model) return;
      setUpdatingFilename(agent.filename);
      try {
        await updateAgent(agent.filename, model, agent.tools === "*");
      } catch {
        // Error is logged in hook
      } finally {
        setUpdatingFilename(null);
      }
    },
    [updateAgent]
  );

  /**
   * Handle all-tools toggle for an agent
   */
  const handleToggleAllTools = useCallback(
    async (agent: Agent) => {
      const currentlyAllTools = agent.tools === "*";
      setUpdatingFilename(agent.filename);
      try {
        await updateAgent(agent.filename, agent.model, !currentlyAllTools);
      } catch {
        // Error is logged in hook
      } finally {
        setUpdatingFilename(null);
      }
    },
    [updateAgent]
  );

  return (
    <Sheet open={open} onOpenChange={onOpenChange}>
      <SheetContent
        side="right"
        className="w-full sm:max-w-lg md:max-w-xl bg-[#0a0a0a] border-zinc-800 flex flex-col"
      >
        <SheetHeader className="space-y-1">
          <SheetTitle className="flex items-center gap-2 text-zinc-100">
            <Bot className="size-5" aria-hidden="true" />
            Agents
          </SheetTitle>
          <SheetDescription className="text-zinc-500">
            {isLoading
              ? "Loading..."
              : `${agents.length} ${agents.length === 1 ? "agent" : "agents"}`}
          </SheetDescription>
        </SheetHeader>

        {/* Agents list */}
        <ScrollArea className="flex-1 mt-4 -mx-6 px-6">
          <div className="space-y-2 pb-4">
            {isLoading ? (
              <div className="flex items-center justify-center py-12">
                <Loader2
                  className="size-5 text-zinc-500 animate-spin"
                  aria-hidden="true"
                />
                <span className="sr-only">Loading agents</span>
              </div>
            ) : error ? (
              <div
                role="alert"
                className="rounded-lg border border-red-500/30 bg-red-500/10 p-4 text-center"
              >
                <p className="text-sm text-red-400">
                  Failed to load agents
                </p>
                <p className="text-xs text-red-400/60 mt-1">
                  {error.message}
                </p>
              </div>
            ) : agents.length === 0 ? (
              <div className="flex flex-col items-center justify-center py-12 text-center">
                <Bot
                  className="size-8 text-zinc-700 mb-3"
                  aria-hidden="true"
                />
                <p className="text-sm text-zinc-500">No agents configured</p>
                <p className="text-xs text-zinc-600 mt-1">
                  Add agent files to .claude/agents/ to get started
                </p>
              </div>
            ) : (
              agents.map((agent) => (
                <AgentCard
                  key={agent.filename}
                  agent={agent}
                  isExpanded={expandedFilename === agent.filename}
                  onToggle={() => handleToggle(agent.filename)}
                  onUpdateModel={(model) => handleUpdateModel(agent, model)}
                  onToggleAllTools={() => handleToggleAllTools(agent)}
                  isUpdating={updatingFilename === agent.filename}
                />
              ))
            )}
          </div>
        </ScrollArea>
      </SheetContent>
    </Sheet>
  );
}
