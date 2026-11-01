<script lang="ts">
  import type { Component } from "svelte";
  import { page } from "$app/state";
  import {
    CloudUpload,
    Bot,
    CircleUser,
    Github,
    Info,
    Keyboard,
    Languages,
    SlidersHorizontal,
    SquareTerminal,
  } from "@lucide/svelte";
  import AboutSection from "$lib/components/settings/AboutSection.svelte";
  import AccountSection from "$lib/components/settings/AccountSection.svelte";
  import AgentAppsSection from "$lib/components/settings/AgentAppsSection.svelte";
  import CliSection from "$lib/components/settings/CliSection.svelte";
  import GeneralSection from "$lib/components/settings/GeneralSection.svelte";
  import GithubSection from "$lib/components/settings/GithubSection.svelte";
  import LibrarySyncSection from "$lib/components/settings/LibrarySyncSection.svelte";
  import ShortcutsSection from "$lib/components/settings/ShortcutsSection.svelte";
  import TranslationSection from "$lib/components/settings/TranslationSection.svelte";
  import { t } from "$lib/i18n";
  import {
    buildSettingsHref,
    parseSettingsSection,
    type SettingsSection,
  } from "$lib/navigation/app-shell";

  const section = $derived(parseSettingsSection(page.url.searchParams.get("section")));

  const sections: { key: SettingsSection; label: string; icon: Component<{ size?: number }> }[] =
    $derived([
      { key: "general", label: $t("settings.section.general"), icon: SlidersHorizontal },
      { key: "account", label: $t("settings.account.title"), icon: CircleUser },
      { key: "agents", label: $t("agentApps.title"), icon: Bot },
      { key: "translation", label: $t("settings.translation.title"), icon: Languages },
      { key: "github", label: $t("settings.github.title"), icon: Github },
      { key: "backup", label: $t("settings.section.backup"), icon: CloudUpload },
      { key: "shortcuts", label: $t("settings.shortcuts.title"), icon: Keyboard },
      { key: "cli", label: $t("settings.cli.title"), icon: SquareTerminal },
      { key: "about", label: $t("settings.section.about"), icon: Info },
    ]);
</script>

<section class="flex h-full min-h-0 min-w-0 flex-col overflow-hidden">
  <!-- A fixed, narrow nav: the topics are short and few, unlike the resizable lists. -->
  <div class="grid min-h-0 flex-1 grid-cols-[13rem_minmax(0,1fr)]">
    <div class="flex min-h-0 min-w-0 flex-col">
      <header class="flex h-12 flex-none items-center px-4" data-window-drag-region>
        <h1 class="text-base-content truncate text-[1.05rem] font-semibold tracking-[-0.02em]">
          {$t("header.settings")}
        </h1>
      </header>
      <nav class="min-h-0 flex-1 overflow-y-auto py-1" aria-label={$t("header.settings")}>
        {#each sections as item (item.key)}
          {@const selected = item.key === section}
          {@const Icon = item.icon}
          <a
            class={`mx-1.5 mb-0.5 flex items-center gap-2.5 rounded-lg px-2.5 py-2 text-[13px] transition ${
              selected
                ? "bg-base-content/[0.07] text-base-content font-medium"
                : "text-base-content-muted hover:bg-base-content/[0.045] hover:text-base-content"
            }`}
            href={buildSettingsHref(item.key)}
            data-sveltekit-replacestate
            data-sveltekit-noscroll
            aria-current={selected ? "page" : undefined}
          >
            <Icon size={15} />
            <span class="truncate">{item.label}</span>
          </a>
        {/each}
      </nav>
    </div>

    <div class="surface-panel grid min-h-0 min-w-0 grid-rows-[minmax(0,1fr)]">
      {#key section}
        {#if section === "account"}
          <AccountSection />
        {:else if section === "agents"}
          <AgentAppsSection />
        {:else if section === "translation"}
          <TranslationSection />
        {:else if section === "github"}
          <GithubSection />
        {:else if section === "backup"}
          <LibrarySyncSection />
        {:else if section === "shortcuts"}
          <ShortcutsSection />
        {:else if section === "cli"}
          <CliSection />
        {:else if section === "about"}
          <AboutSection />
        {:else}
          <GeneralSection />
        {/if}
      {/key}
    </div>
  </div>
</section>
