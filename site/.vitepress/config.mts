import { defineConfig } from 'vitepress'

// https://vitepress.dev/reference/site-config
export default defineConfig({
  title: "Probe",
  description: "AI-friendly, fully local, semantic code search tool designed to power the next generation of AI coding assistants",
  lastUpdated: true,
  cleanUrls: true,
  appearance: true,
  ignoreDeadLinks: true,
  
  // Site URL for sitemap and canonical URLs
  sitemap: {
    hostname: 'https://probelabs.com'
  },
  
  // Site metadata for probelabs.com
  head: [
    ['link', { rel: 'canonical', href: 'https://probelabs.com/' }],
    ['meta', { property: 'og:site_name', content: 'Probe Labs' }],
    ['meta', { property: 'og:url', content: 'https://probelabs.com/' }],
    ['meta', { property: 'twitter:site', content: '@buger' }],
    ['meta', { name: 'robots', content: 'index,follow' }]
  ],
  
  markdown: {
    theme: {
      light: 'github-light',
      dark: 'github-dark'
    },
    lineNumbers: true
  },
  
  vue: {
    template: {
      compilerOptions: {
        isCustomElement: (tag) => tag.includes('-')
      }
    }
  },
  
  themeConfig: {
    darkModeSwitchLabel: 'Appearance',
    lightModeSwitchTitle: 'Switch to light theme',
    darkModeSwitchTitle: 'Switch to dark theme',
    
    nav: [
      { text: 'Home', link: '/' },
      { text: 'Quick Start', link: '/quick-start' },
      { text: 'Blog', link: '/blog/' },
      { text: 'Changelog', link: '/changelog' },
      {
        text: 'Documentation',
        items: [
          { text: 'Core Concepts & Setup', items: [
            { text: 'What is Probe?', link: '/features' },
            { text: 'Installation', link: '/installation' },
            { text: 'Quick Start', link: '/quick-start' },
            { text: 'Configuration', link: '/configuration' },
            { text: 'Language Support', link: '/language-support-overview' },
            { text: 'Supported Languages', link: '/supported-languages' },
            { text: 'How It Works', link: '/how-it-works' },
          ]},
          { text: 'Use Cases / Workflows', items: [
            { text: 'AI Code Editors & MCP', link: '/use-cases/integrating-probe-into-ai-code-editors' },
            { text: 'CLI for AI Workflows', link: '/use-cases/advanced-cli' },
            { text: 'Web Interface for Teams', link: '/use-cases/deploying-probe-web-interface' },
            { text: 'Developers & SDK', link: '/use-cases/building-ai-tools' },
          ]},
          { text: 'Reference Documentation', items: [
            { text: 'CLI Reference (Commands & Flags)', link: '/cli-mode' },
            { text: 'AI Integration Overview', link: '/ai-integration' },
            { text: 'AI Chat Mode', link: '/ai-chat' },
            { text: 'Web Interface', link: '/web-interface' },
            { text: 'MCP Protocol & Tools Reference', link: '/mcp-server' },
            { text: 'Node.js SDK API Reference', link: '/nodejs-sdk' },
            { text: 'Output Formats Reference', link: '/output-formats' },
            { text: 'Search Reference', link: '/search-reference' },
            { text: 'Code Extraction', link: '/code-extraction' },
            { text: 'Adding Languages', link: '/adding-languages' },
          ]},
          { text: 'LSP Indexing System', items: [
            { text: 'LSP Features Overview', link: '/lsp-features' },
            { text: 'Indexing Overview', link: '/indexing-overview' },
            { text: 'Architecture Guide', link: '/indexing-architecture' },
            { text: 'Configuration Reference', link: '/indexing-configuration' },
            { text: 'CLI Reference', link: '/indexing-cli-reference' },
            { text: 'Language-Specific Guide', link: '/indexing-languages' },
            { text: 'Performance Guide', link: '/indexing-performance' },
            { text: 'API Reference', link: '/indexing-api-reference' },
          ]}
        ]
      },
      { text: 'GitHub', link: 'https://github.com/buger/probe' },
      { text: 'Discord', link: 'https://discord.gg/hBN4UsTZ' }
    ],

    sidebar: [
      {
        text: 'Core Concepts & Setup',
        collapsed: false,
        items: [
          { text: 'What is Probe?', link: '/features' },
          { text: 'Installation', link: '/installation' },
          { text: 'Quick Start', link: '/quick-start' },
          { text: 'Configuration', link: '/configuration' },
          { text: 'Language Support', link: '/language-support-overview' },
          { text: 'Supported Languages', link: '/supported-languages' },
          { text: 'How It Works', link: '/how-it-works' },
          { text: 'Examples', link: '/examples' }
        ]
      },
      {
        text: 'Use Cases / Workflows',
        collapsed: false,
        items: [
          { text: 'AI Code Editors & MCP', link: '/use-cases/integrating-probe-into-ai-code-editors' },
          { text: 'CLI for AI Workflows', link: '/use-cases/advanced-cli' },
          { text: 'Web Interface for Teams', link: '/use-cases/deploying-probe-web-interface' },
          { text: 'Developers & SDK', link: '/use-cases/building-ai-tools' },
          { text: 'GitHub Actions Integration', link: '/integrations/github-actions' }
        ]
      },
      {
        text: 'Reference Documentation',
        collapsed: false,
        items: [
          { text: 'CLI Reference (Commands & Flags)', link: '/cli-mode' },
          { text: 'AI Integration Overview', link: '/ai-integration' },
          { text: 'AI Chat Mode', link: '/ai-chat' },
          { text: 'Web Interface', link: '/web-interface' },
          { text: 'MCP Protocol & Tools Reference', link: '/mcp-server' },
          { text: 'Node.js SDK API Reference', link: '/nodejs-sdk' },
          { text: 'Output Formats Reference', link: '/output-formats' },
          { text: 'Search Reference', link: '/search-reference' },
          { text: 'Code Extraction', link: '/code-extraction' },
          { text: 'Adding Languages', link: '/adding-languages' },
        ]
      },
      {
        text: 'LSP Indexing System',
        collapsed: false,
        items: [
          { text: 'LSP Features Overview', link: '/lsp-features' },
          { text: 'Indexing Overview', link: '/indexing-overview' },
          { text: 'Architecture Guide', link: '/indexing-architecture' },
          { text: 'Configuration Reference', link: '/indexing-configuration' },
          { text: 'CLI Reference', link: '/indexing-cli-reference' },
          { text: 'Language-Specific Guide', link: '/indexing-languages' },
          { text: 'Performance Guide', link: '/indexing-performance' },
          { text: 'API Reference', link: '/indexing-api-reference' },
        ]
      },
      {
        text: 'Contributing',
        collapsed: true,
        items: [
          { text: 'Contributing Guide', link: 'https://github.com/buger/probe/blob/main/CONTRIBUTING.md' },
          { text: 'Code of Conduct', link: 'https://github.com/buger/probe/blob/main/CODE_OF_CONDUCT.md' },
          { text: 'Documentation Maintenance', link: '/contributing/documentation-maintenance' },
          { text: 'Documentation Structure', link: '/contributing/documentation-structure' },
          { text: 'Documentation Cross-References', link: '/contributing/documentation-cross-references' }
        ]
      },
      {
        text: 'Release Information',
        collapsed: true,
        items: [
          { text: 'Changelog', link: '/changelog' },
          { text: 'Blog', link: '/blog/' },
          { text: 'GitHub Releases', link: 'https://github.com/buger/probe/releases' }
        ]
      }
    ],

    socialLinks: [
      { icon: 'github', link: 'https://github.com/buger/probe' },
      { icon: 'discord', link: 'https://discord.gg/hBN4UsTZ' }
    ],
    
    footer: {
      message: 'Released under the Apache 2.0 License.',
      copyright: 'Copyright © 2025 Leonid Bugaev <div style="display:inline-block"><div class="VPSocialLinks" style="display:inline-flex;gap:8px;margin-left:4px"><a class="VPSocialLink" href="https://github.com/buger" target="_blank" aria-label="github"><svg xmlns="http://www.w3.org/2000/svg" width="20" height="20" viewBox="0 0 24 24"><path fill="currentColor" d="M12 2C6.475 2 2 6.475 2 12a9.994 9.994 0 0 0 6.838 9.488c.5.087.687-.213.687-.476c0-.237-.013-1.024-.013-1.862c-2.512.463-3.162-.612-3.362-1.175c-.113-.288-.6-1.175-1.025-1.413c-.35-.187-.85-.65-.013-.662c.788-.013 1.35.725 1.538 1.025c.9 1.512 2.338 1.087 2.912.825c.088-.65.35-1.087.638-1.337c-2.225-.25-4.55-1.113-4.55-4.938c0-1.088.387-1.987 1.025-2.688c-.1-.25-.45-1.275.1-2.65c0 0 .837-.262 2.75 1.026a9.28 9.28 0 0 1 2.5-.338c.85 0 1.7.112 2.5.337c1.912-1.3 2.75-1.024 2.75-1.024c.55 1.375.2 2.4.1 2.65c.637.7 1.025 1.587 1.025 2.687c0 3.838-2.337 4.688-4.562 4.938c.362.312.675.912.675 1.85c0 1.337-.013 2.412-.013 2.75c0 .262.188.574.688.474A10.016 10.016 0 0 0 22 12c0-5.525-4.475-10-10-10z"/></svg></a><a class="VPSocialLink" href="https://www.linkedin.com/in/leonidbugaev/" target="_blank" aria-label="linkedin"><svg xmlns="http://www.w3.org/2000/svg" width="20" height="20" viewBox="0 0 24 24"><path fill="currentColor" d="M19 3a2 2 0 0 1 2 2v14a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h14m-.5 15.5v-5.3a3.26 3.26 0 0 0-3.26-3.26c-.85 0-1.84.52-2.32 1.3v-1.11h-2.79v8.37h2.79v-4.93c0-.77.62-1.4 1.39-1.4a1.4 1.4 0 0 1 1.4 1.4v4.93h2.79M6.88 8.56a1.68 1.68 0 0 0 1.68-1.68c0-.93-.75-1.69-1.68-1.69a1.69 1.69 0 0 0-1.69 1.69c0 .93.76 1.68 1.69 1.68m1.39 9.94v-8.37H5.5v8.37h2.77z"/></svg></a><a class="VPSocialLink" href="https://x.com/buger" target="_blank" aria-label="twitter"><svg xmlns="http://www.w3.org/2000/svg" width="20" height="20" viewBox="0 0 24 24"><path fill="currentColor" d="M22.46 6c-.77.35-1.6.58-2.46.69c.88-.53 1.56-1.37 1.88-2.38c-.83.5-1.75.85-2.72 1.05C18.37 4.5 17.26 4 16 4c-2.35 0-4.27 1.92-4.27 4.29c0 .34.04.67.11.98C8.28 9.09 5.11 7.38 3 4.79c-.37.63-.58 1.37-.58 2.15c0 1.49.75 2.81 1.91 3.56c-.71 0-1.37-.2-1.95-.5v.03c0 2.08 1.48 3.82 3.44 4.21a4.22 4.22 0 0 1-1.93.07a4.28 4.28 0 0 0 4 2.98a8.521 8.521 0 0 1-5.33 1.84c-.34 0-.68-.02-1.02-.06C3.44 20.29 5.7 21 8.12 21C16 21 20.33 14.46 20.33 8.79c0-.19 0-.37-.01-.56c.84-.6 1.56-1.36 2.14-2.23z"/></svg></a></div></div>'
    },
    
    search: {
      provider: 'local'
    },
    
    outline: {
      level: [2, 3],
      label: 'On this page'
    }
  }
})
