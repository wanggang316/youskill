<script lang="ts">
  import { tick } from "svelte";
  import { X } from "@lucide/svelte";
  import IconButton from "./IconButton.svelte";

  interface Props {
    open?: boolean;
    title?: string;
    showCloseButton?: boolean;
    closeOnBackdropClick?: boolean;
    onClose?: () => void;
    children?: import("svelte").Snippet;
    footer?: import("svelte").Snippet;
    containerClass?: string;
    /** `top` keeps the dialog near the top, so it does not move while its content grows. */
    placement?: "center" | "top";
  }

  let {
    open = $bindable(false),
    title = "",
    showCloseButton = true,
    closeOnBackdropClick = false,
    onClose = () => {},
    children,
    footer,
    containerClass = "",
    placement = "center",
  }: Props = $props();
  const resolvedContainerClass = $derived(containerClass || "max-w-[min(90vw,56rem)]");

  let closing = $state(false);
  let modalElement = $state<HTMLDivElement>();

  export function handleClose() {
    closing = true;
    setTimeout(() => {
      closing = false;
      open = false;
      onClose();
    }, 250);
  }

  function handleBackdropClick(e: MouseEvent) {
    if (closeOnBackdropClick && e.target === e.currentTarget) {
      handleClose();
    }
  }

  $effect(() => {
    if (open && modalElement) {
      tick().then(() => modalElement?.focus());
    }
  });
</script>

{#if open}
  <div
    bind:this={modalElement}
    class={`animate-backdrop fixed inset-0 z-(--z-modal) flex justify-center p-4 ${
      placement === "top" ? "items-start pt-[12vh]" : "items-center"
    }`}
    style="background-color: var(--overlay);"
    class:animate-backdrop-close={closing}
    role="dialog"
    aria-modal="true"
    tabindex="-1"
    onclick={handleBackdropClick}
    onkeydown={(e) => {
      if (e.key === "Escape") handleClose();
    }}
  >
    <div
      class={`animate-modal bg-base-100 border-base-200 relative flex max-h-[90vh] w-full flex-col overflow-hidden rounded-2xl border shadow-2xl ${resolvedContainerClass}`}
      class:animate-modal-close={closing}
    >
      {#if showCloseButton || title}
        <div
          class="bg-base-100/95 border-base-200 sticky top-0 z-20 flex shrink-0 items-center justify-between gap-3 border-b px-5 py-2 backdrop-blur-sm"
        >
          {#if title}
            <h3 class="text-base-content text-base font-medium">
              {title}
            </h3>
          {/if}
          {#if showCloseButton}
            <IconButton onclick={handleClose} ariaLabel="Close" title="Close">
              <X size={20} />
            </IconButton>
          {/if}
        </div>
      {/if}

      <div class="min-h-0 flex-1 overflow-y-auto">
        {#if children}
          {@render children()}
        {/if}
      </div>
      {#if footer}
        <div
          class="bg-base-100 border-base-200 flex shrink-0 items-center justify-end gap-3 border-t px-6 py-3"
        >
          {@render footer()}
        </div>
      {/if}
    </div>
  </div>
{/if}

<style>
  .animate-backdrop {
    animation: backdropFadeIn 0.25s ease-out;
  }

  .animate-backdrop-close {
    animation: backdropFadeOut 0.25s ease-out;
  }

  .animate-modal {
    animation: modalSlideIn 0.25s ease-out;
  }

  .animate-modal-close {
    animation: modalSlideOut 0.25s ease-out;
  }

  @keyframes backdropFadeIn {
    from {
      opacity: 0;
    }
    to {
      opacity: 1;
    }
  }

  @keyframes backdropFadeOut {
    from {
      opacity: 1;
    }
    to {
      opacity: 0;
    }
  }

  @keyframes modalSlideIn {
    from {
      opacity: 0;
      transform: translateY(-30px);
    }
    to {
      opacity: 1;
      transform: translateY(0);
    }
  }

  @keyframes modalSlideOut {
    from {
      opacity: 1;
      transform: translateY(0);
    }
    to {
      opacity: 0;
      transform: translateY(-30px);
    }
  }
</style>
