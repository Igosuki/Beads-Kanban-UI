"use client";

import { useState } from "react";

import { Loader2 } from "lucide-react";
import ReactMarkdown from "react-markdown";

import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { useToast } from "@/hooks/use-toast";
import { createBead } from "@/lib/cli";

type BeadType = "task" | "epic";

interface CreateBeadDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  projectPath?: string;
  onCreated?: () => void;
}

export function CreateBeadDialog({
  open,
  onOpenChange,
  projectPath,
  onCreated,
}: CreateBeadDialogProps) {
  const [title, setTitle] = useState("");
  const [description, setDescription] = useState("");
  const [beadType, setBeadType] = useState<BeadType>("task");
  const [isSubmitting, setIsSubmitting] = useState(false);
  const [previewMode, setPreviewMode] = useState(false);
  const { toast } = useToast();

  const resetForm = () => {
    setTitle("");
    setDescription("");
    setBeadType("task");
    setPreviewMode(false);
  };

  const handleOpenChange = (nextOpen: boolean) => {
    if (!nextOpen) {
      resetForm();
    }
    onOpenChange(nextOpen);
  };

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();

    if (!title.trim()) {
      return;
    }

    setIsSubmitting(true);

    try {
      const beadId = await createBead(title.trim(), description.trim(), projectPath, { type: beadType });

      toast({
        title: "Bead created",
        description: beadId
          ? `Created bead ${beadId} successfully.`
          : "Bead created successfully.",
      });

      resetForm();
      onOpenChange(false);
      onCreated?.();
    } catch (err) {
      console.error("Error creating bead:", err);
      toast({
        title: "Error",
        description:
          err instanceof Error ? err.message : "Failed to create bead. Please try again.",
        variant: "destructive",
      });
    } finally {
      setIsSubmitting(false);
    }
  };

  return (
    <Dialog open={open} onOpenChange={handleOpenChange}>
      <DialogContent className="sm:max-w-2xl">
        <DialogHeader>
          <DialogTitle>Create Bead</DialogTitle>
          <DialogDescription>
            Add a new task or epic to the board.
          </DialogDescription>
        </DialogHeader>

        <form onSubmit={handleSubmit}>
          <div className="space-y-4 py-4">
            <div className="space-y-2">
              <label htmlFor="bead-title" className="text-sm font-medium text-zinc-300">
                Title <span className="text-red-400">*</span>
              </label>
              <Input
                id="bead-title"
                value={title}
                onChange={(e) => setTitle(e.target.value)}
                placeholder="Enter bead title"
                autoFocus
                required
              />
            </div>

            <div className="space-y-2">
              <div className="flex items-center justify-between">
                <label htmlFor="bead-description" className="text-sm font-medium text-zinc-300">
                  Description <span className="text-xs text-zinc-500 ml-1">Markdown supported</span>
                </label>
                <div className="flex gap-1">
                  <button
                    type="button"
                    className={`px-2 py-0.5 text-xs rounded ${!previewMode ? "bg-zinc-700 text-zinc-200" : "text-zinc-500 hover:text-zinc-300"}`}
                    onClick={() => setPreviewMode(false)}
                  >
                    Write
                  </button>
                  <button
                    type="button"
                    className={`px-2 py-0.5 text-xs rounded ${previewMode ? "bg-zinc-700 text-zinc-200" : "text-zinc-500 hover:text-zinc-300"}`}
                    onClick={() => setPreviewMode(true)}
                  >
                    Preview
                  </button>
                </div>
              </div>
              {previewMode ? (
                <div className="min-h-[200px] max-h-[400px] overflow-y-auto rounded-md border border-input bg-transparent px-3 py-2 text-sm prose prose-invert prose-sm max-w-none">
                  {description.trim() ? (
                    <ReactMarkdown>{description}</ReactMarkdown>
                  ) : (
                    <p className="text-muted-foreground italic">Nothing to preview</p>
                  )}
                </div>
              ) : (
                <textarea
                  id="bead-description"
                  value={description}
                  onChange={(e) => setDescription(e.target.value)}
                  placeholder="Optional description (Markdown supported)"
                  className="flex w-full min-h-[200px] max-h-[400px] rounded-md border border-input bg-transparent px-3 py-2 text-sm font-mono shadow-sm transition-colors placeholder:text-muted-foreground focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-50 resize-y"
                />
              )}
            </div>

            <div className="space-y-2">
              <span className="text-sm font-medium text-zinc-300">Type</span>
              <div className="flex gap-2">
                <Button
                  type="button"
                  variant={beadType === "task" ? "primary" : "outline"}
                  size="sm"
                  onClick={() => setBeadType("task")}
                >
                  Task
                </Button>
                <Button
                  type="button"
                  variant={beadType === "epic" ? "primary" : "outline"}
                  size="sm"
                  onClick={() => setBeadType("epic")}
                >
                  Epic
                </Button>
              </div>
            </div>
          </div>

          <DialogFooter>
            <Button
              type="button"
              variant="outline"
              onClick={() => handleOpenChange(false)}
              disabled={isSubmitting}
            >
              Cancel
            </Button>
            <Button type="submit" disabled={isSubmitting || !title.trim()}>
              {isSubmitting ? (
                <>
                  <Loader2 className="size-4 animate-spin" aria-hidden="true" />
                  Creating...
                </>
              ) : (
                "Create"
              )}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
