"use client";

import { useState } from "react";

import { Loader2 } from "lucide-react";

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
  const [isSubmitting, setIsSubmitting] = useState(false);
  const { toast } = useToast();

  const resetForm = () => {
    setTitle("");
    setDescription("");
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
      const beadId = await createBead(title.trim(), description.trim(), projectPath);

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
      <DialogContent className="sm:max-w-md">
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
              <label htmlFor="bead-description" className="text-sm font-medium text-zinc-300">
                Description
              </label>
              <textarea
                id="bead-description"
                value={description}
                onChange={(e) => setDescription(e.target.value)}
                placeholder="Optional description"
                rows={4}
                className="flex w-full rounded-md border border-input bg-transparent px-3 py-2 text-sm shadow-sm transition-colors placeholder:text-muted-foreground focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-50 resize-none"
              />
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
