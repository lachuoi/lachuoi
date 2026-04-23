(function() {
    try {
        const logsDiv = document.getElementById('logs');
        const statusIndicator = document.getElementById('status-indicator');
        const tableBody = document.getElementById('task-table-body');
        const webhookTableBody = document.getElementById('webhook-table-body');
        
        // --- Theme Logic (EARLY) ---
        const themeToggleBtn = document.getElementById('theme-toggle');
        const themeToggleDarkIcon = document.getElementById('theme-toggle-dark-icon');
        const themeToggleLightIcon = document.getElementById('theme-toggle-light-icon');

        const isDarkMode = localStorage.getItem('color-theme') === 'dark' || (!('color-theme' in localStorage) && window.matchMedia('(prefers-color-scheme: dark)').matches);

        if (isDarkMode) {
            document.documentElement.classList.add('dark');
            if (themeToggleLightIcon) themeToggleLightIcon.classList.remove('hidden');
            if (themeToggleDarkIcon) themeToggleDarkIcon.classList.add('hidden');
        } else {
            document.documentElement.classList.remove('dark');
            if (themeToggleDarkIcon) themeToggleDarkIcon.classList.remove('hidden');
            if (themeToggleLightIcon) themeToggleLightIcon.classList.add('hidden');
        }

        themeToggleBtn?.addEventListener('click', () => {
            if (themeToggleDarkIcon) themeToggleDarkIcon.classList.toggle('hidden');
            if (themeToggleLightIcon) themeToggleLightIcon.classList.toggle('hidden');

            if (localStorage.getItem('color-theme')) {
                if (localStorage.getItem('color-theme') === 'light') {
                    document.documentElement.classList.add('dark');
                    localStorage.setItem('color-theme', 'dark');
                } else {
                    document.documentElement.classList.remove('dark');
                    localStorage.setItem('color-theme', 'light');
                }
            } else {
                if (document.documentElement.classList.contains('dark')) {
                    document.documentElement.classList.remove('dark');
                    localStorage.setItem('color-theme', 'light');
                } else {
                    document.documentElement.classList.add('dark');
                    localStorage.setItem('color-theme', 'dark');
                }
            }
        });

        // --- SSE Setup ---
        const eventSource = new EventSource('/events');

        // --- Initial Logs ---
        async function fetchInitialLogs() {
            if (!logsDiv) return;
            // If we already have persistent logs, we can skip fetching from DB or just append
            // For now, let's load persistent logs first, then fetch initial if empty
            const saved = localStorage.getItem('lachuoi-persistent-logs');
            if (saved && JSON.parse(saved).length > 0) {
                loadPersistentLogs();
                return;
            }

            try {
                const response = await fetch('/logs/initial');
                if (response.ok) {
                    const data = await response.json();
                    data.logs.forEach(log => {
                        const entry = document.createElement('div');
                        entry.className = 'mb-1 flex gap-3 text-xs md:text-sm log-entry';
                        if (log.task_id) {
                            entry.setAttribute('data-task-id', log.task_id);
                        }
                        const isError = log.output.toLowerCase().includes('failed') || log.output.toLowerCase().includes('error');
                        const textColor = isError ? 'text-red-400 font-medium' : 'text-slate-300';
                        
                        let timeStr;
                        try {
                            const date = new Date(log.created_at);
                            timeStr = isNaN(date.getTime()) ? 'Recent' : date.toLocaleTimeString();
                        } catch(e) {
                            timeStr = 'Recent';
                        }

                        entry.innerHTML = `<span class="text-slate-500 shrink-0 font-medium select-none">[${timeStr}]</span><span class="${textColor}">${log.output}</span>`;
                        logsDiv.appendChild(entry);
                    });
                    logsDiv.scrollTop = logsDiv.scrollHeight;
                    setTimeout(() => {
                        logsDiv.scrollTop = logsDiv.scrollHeight;
                    }, 50);
                }
            } catch (error) {
                console.error('Error fetching initial logs:', error);
            }
        }

        fetchInitialLogs();

        // --- Layout Logic ---
        const layoutToggleBtn = document.getElementById('layout-toggle');
        const layoutHorizontalIcon = document.getElementById('layout-horizontal-icon');
        const layoutVerticalIcon = document.getElementById('layout-vertical-icon');
        const mainContainer = document.getElementById('main-container');
        const tasksSection = document.getElementById('tasks-section') || document.getElementById('webhook-logs-container');
        const logsSection = document.getElementById('logs-section');
        const pageContainer = document.getElementById('page-container');
        const headerContainer = document.getElementById('header-container');

        function applyLayout(layout) {
            if (!mainContainer) return;

            // Only allow split layout if both sections exist
            const canSplit = tasksSection && logsSection;
            const targetLayout = canSplit ? layout : 'stack';

            if (targetLayout === 'split') {
                mainContainer.classList.remove('flex-col', 'items-center');
                mainContainer.classList.add('md:flex-row', 'items-start', 'w-full');
                if (headerContainer) headerContainer.classList.remove('max-w-[80%]', 'mx-auto');
                if (tasksSection) {
                    tasksSection.classList.remove('max-w-[80%]', 'w-full');
                    tasksSection.classList.add('md:flex-[6]', 'min-w-0');
                }
                if (logsSection) {
                    logsSection.classList.remove('max-w-[80%]', 'w-full');
                    logsSection.classList.add('md:flex-[4]', 'min-w-0');
                }
                if (pageContainer) {
                    pageContainer.classList.remove('max-w-6xl', 'md:max-w-[98%]');
                    pageContainer.classList.add('md:max-w-[98%]');
                }
                if (layoutHorizontalIcon) layoutHorizontalIcon.classList.add('hidden');
                if (layoutVerticalIcon) layoutVerticalIcon.classList.remove('hidden');
                if (logsDiv) logsDiv.style.height = 'calc(100vh - 250px)';
            } else {
                mainContainer.classList.add('flex-col', 'items-center');
                mainContainer.classList.remove('md:flex-row', 'items-start', 'w-full');
                if (headerContainer) headerContainer.classList.add('max-w-[80%]', 'mx-auto');
                if (tasksSection) {
                    tasksSection.classList.remove('md:flex-[6]', 'md:w-2/3', 'min-w-0');
                    tasksSection.classList.add('max-w-[80%]', 'w-full');
                }
                if (logsSection) {
                    logsSection.classList.remove('md:flex-[4]', 'md:w-1/3', 'min-w-0');
                    logsSection.classList.add('max-w-[80%]', 'w-full');
                }
                if (pageContainer) {
                    pageContainer.classList.remove('max-w-6xl', 'md:max-w-[98%]');
                }
                if (layoutHorizontalIcon) layoutHorizontalIcon.classList.remove('hidden');
                if (layoutVerticalIcon) layoutVerticalIcon.classList.add('hidden');
                if (logsDiv) logsDiv.style.height = '400px';
            }
        }

        // Initialize layout
        const currentLayout = localStorage.getItem('dashboard-layout') || 'stack';
        applyLayout(currentLayout);

        layoutToggleBtn?.addEventListener('click', () => {
            const newLayout = localStorage.getItem('dashboard-layout') === 'split' ? 'stack' : 'split';
            localStorage.setItem('dashboard-layout', newLayout);
            applyLayout(newLayout);
        });

        // --- Table & Sorting Logic ---
        let latestTasks = [];
        try {
            const savedTasks = localStorage.getItem('lachuoi-latest-tasks');
            if (savedTasks) {
                latestTasks = JSON.parse(savedTasks);
            }
        } catch (e) { console.error("Error loading tasks from localStorage:", e); }

        let currentSortColumn = 'name';
        let currentSortDirection = 'asc';

        // Initial render if we have saved tasks
        if (latestTasks.length > 0) {
            setTimeout(renderTable, 0);
        }

        window.setSort = function(column) {
            if (currentSortColumn === column) {
                currentSortDirection = currentSortDirection === 'asc' ? 'desc' : 'asc';
            } else {
                currentSortColumn = column;
                currentSortDirection = 'asc';
            }
            renderTable();
        };

        function updateSortIndicators() {
            const columns = ['name', 'task_type', 'cron', 'timezone', 'last_run', 'enabled'];
            columns.forEach(col => {
                const indicator = document.getElementById(`sort-${col}`);
                if (!indicator) return;
                
                if (currentSortColumn === col) {
                    indicator.innerText = currentSortDirection === 'asc' ? '↑' : '↓';
                    indicator.className = 'text-blue-500 dark:text-blue-400 font-bold opacity-100';
                } else {
                    indicator.innerText = '↕';
                    indicator.className = 'opacity-0 group-hover:opacity-100 transition-opacity';
                }
            });
        }

        function renderTable() {
            if (!latestTasks || latestTasks.length === 0 || !tableBody) return;

            const sortedTasks = [...latestTasks].sort((a, b) => {
                let valA = a[currentSortColumn];
                let valB = b[currentSortColumn];
                if (currentSortColumn === 'last_run') {
                    valA = valA ? new Date(valA).getTime() : 0;
                    valB = valB ? new Date(valB).getTime() : 0;
                } else {
                    if (valA === null || valA === undefined) valA = '';
                    if (valB === null || valB === undefined) valB = '';
                }
                if (valA < valB) return currentSortDirection === 'asc' ? -1 : 1;
                if (valA > valB) return currentSortDirection === 'asc' ? 1 : -1;
                return 0;
            });

            let rows = '';
            sortedTasks.forEach(task => {
                let statusClass = task.enabled 
                    ? 'bg-green-50 text-green-700 border-green-200 dark:bg-green-900/20 dark:text-green-400 dark:border-green-800' 
                    : 'bg-red-50 text-red-700 border-red-200 dark:bg-red-900/20 dark:text-red-400 dark:border-red-800';
                let statusText = task.enabled ? 'Active' : 'Paused';
                let statusOnclick = '';
                if (task.enabled && task.last_failed) {
                    statusClass = 'bg-red-50 text-red-700 border-red-200 dark:bg-red-900/20 dark:text-red-400 dark:border-red-800 cursor-pointer hover:bg-red-100 dark:hover:bg-red-900/40';
                    statusText = 'Failed';
                    if (task.last_log_id) {
                        statusOnclick = `onclick='showTaskLogs("${task.last_log_id}")'`;
                    }
                }
                const lastRunStr = formatRelativeTime(task.last_run);
                const durationStr = (task.last_duration_ms !== null && task.last_duration_ms !== undefined) 
                    ? `<span class='ml-1 text-xs text-blue-600 dark:text-blue-400 font-bold'>(${task.last_duration_ms}ms)</span>` 
                    : '';
                
                const runBtn = task.enabled 
                    ? `<button class='px-3 py-1 text-xs font-semibold text-blue-600 bg-blue-50 border border-blue-200 rounded-md hover:bg-blue-600 hover:text-white dark:bg-blue-900/20 dark:border-blue-800 dark:hover:bg-blue-600 transition-all duration-200 mr-2' onclick='runTask("${task.id}")'>Run</button>`
                    : '';

                const toggleBtn = task.enabled 
                    ? `<button class='px-3 py-1 text-xs font-semibold text-red-600 bg-red-50 border border-red-200 rounded-md hover:bg-red-600 hover:text-white dark:bg-red-900/20 dark:border-red-800 dark:hover:bg-red-600 transition-all duration-200' onclick='toggleTask("${task.id}", false)'>Disable</button>`
                    : `<button class='px-3 py-1 text-xs font-semibold text-green-600 bg-green-50 border border-green-200 rounded-md hover:bg-green-600 hover:text-white dark:bg-green-900/20 dark:border-green-800 dark:hover:bg-green-600 transition-all duration-200' onclick='toggleTask("${task.id}", true)'>Enable</button>`;

                rows += `
                    <tr class="border-b border-gray-100 dark:border-slate-800 hover:bg-gray-50 dark:hover:bg-slate-800/50 transition-colors">
                        <td class="px-4 py-3 align-middle text-sm font-bold text-gray-900 dark:text-slate-100 cursor-pointer hover:text-blue-600 dark:hover:text-blue-400" onclick="filterTask('${task.id}')">${task.name}</td>
                        <td class="px-4 py-3 align-middle text-xs"><span class="bg-gray-100 dark:bg-slate-800 text-gray-600 dark:text-slate-400 px-2 py-1 rounded font-medium uppercase tracking-wider">${task.task_type}</span></td>
                        <td class="px-4 py-3 align-middle font-mono text-blue-600 dark:text-blue-400 text-xs">${task.cron}</td>
                        <td class="px-4 py-3 align-middle text-gray-600 dark:text-slate-400 text-sm">${task.timezone}</td>
                        <td class="px-4 py-3 align-middle text-sm text-gray-500 dark:text-slate-500" title="${task.last_run || 'Never'}">${lastRunStr} ${durationStr}</td>
                        <td class="px-4 py-3 align-middle"><span class="px-2 py-1 text-[10px] uppercase font-bold rounded-full border ${statusClass}" ${statusOnclick}>${statusText}</span></td>
                        <td class="px-4 py-3 align-middle">${runBtn}${toggleBtn}</td>
                    </tr>
                `;
            });
            tableBody.innerHTML = rows;
            updateSortIndicators();
        }

        function formatRelativeTime(lastRunRfc3339) {
            if (!lastRunRfc3339) return 'Never';
            const lastRun = new Date(lastRunRfc3339);
            const now = new Date();
            const diffSeconds = Math.floor((now - lastRun) / 1000);
            if (isNaN(lastRun.getTime())) return 'Invalid Date';
            if (diffSeconds < 0) return 'Just now';
            if (diffSeconds < 60) return `${diffSeconds}s ago`;
            if (diffSeconds < 3600) return `${Math.floor(diffSeconds / 60)}m ago`;
            if (diffSeconds < 86400) return `${Math.floor(diffSeconds / 3600)}h ago`;
            return lastRun.toLocaleTimeString();
        }

        function pruneLogs() {
            if (!logsDiv) return;
            while (logsDiv.childNodes.length > 1000) {
                logsDiv.removeChild(logsDiv.firstChild);
            }
        }

        function saveLogToPersistentStorage(logText, isError, taskId) {
            try {
                let persistentLogs = [];
                const saved = localStorage.getItem('lachuoi-persistent-logs');
                if (saved) persistentLogs = JSON.parse(saved);
                
                persistentLogs.push({
                    text: logText,
                    isError: isError,
                    taskId: taskId,
                    time: new Date().toISOString()
                });
                
                // Keep last 200 logs in persistent storage
                if (persistentLogs.length > 200) {
                    persistentLogs = persistentLogs.slice(-200);
                }
                
                localStorage.setItem('lachuoi-persistent-logs', JSON.stringify(persistentLogs));
            } catch (e) { console.error("Error saving log to localStorage:", e); }
        }

        function loadPersistentLogs() {
            if (!logsDiv) return;
            try {
                const saved = localStorage.getItem('lachuoi-persistent-logs');
                if (saved) {
                    const persistentLogs = JSON.parse(saved);
                    persistentLogs.forEach(log => {
                        const entry = document.createElement('div');
                        entry.className = 'mb-1 flex gap-3 text-xs md:text-sm log-entry';
                        if (log.taskId) entry.setAttribute('data-task-id', log.taskId);
                        const textColor = log.isError ? 'text-red-400 font-medium' : 'text-slate-300';
                        const timeStr = new Date(log.time).toLocaleTimeString();
                        entry.innerHTML = `<span class="text-slate-500 shrink-0 font-medium select-none">[${timeStr}]</span><span class="${textColor}">${log.text}</span>`;
                        logsDiv.appendChild(entry);
                    });
                    logsDiv.scrollTop = logsDiv.scrollHeight;
                    // Double check with a small timeout to ensure layout is complete
                    setTimeout(() => {
                        logsDiv.scrollTop = logsDiv.scrollHeight;
                    }, 50);
                }
            } catch (e) { console.error("Error loading logs from localStorage:", e); }
        }

        let activeTaskIdFilter = null;

        eventSource.onmessage = (event) => {
            try {
                const data = JSON.parse(event.data);
                
                if (data.log) {
                    const taskId = data.task_id;
                    const isError = data.log.toLowerCase().includes('failed') || data.log.toLowerCase().includes('error');
                    saveLogToPersistentStorage(data.log, isError, taskId);

                    if (logsDiv) {
                        const entry = document.createElement('div');
                        entry.className = 'mb-1 flex gap-3 text-xs md:text-sm log-entry';
                        if (taskId) entry.setAttribute('data-task-id', taskId);
                        
                        // Apply filter if active
                        if (activeTaskIdFilter && taskId !== activeTaskIdFilter) {
                            entry.classList.add('hidden');
                        }

                        const textColor = isError ? 'text-red-400 font-medium' : 'text-slate-300';
                        const time = new Date().toLocaleTimeString();
                        entry.innerHTML = `<span class="text-slate-500 shrink-0 font-medium select-none">[${time}]</span><span class="${textColor}">${data.log}</span>`;
                        logsDiv.appendChild(entry);
                        
                        if (!entry.classList.contains('hidden')) {
                            logsDiv.scrollTop = logsDiv.scrollHeight;
                        }
                        pruneLogs();
                    }
                } else if (data.status) {
                    latestTasks = data.status;
                    localStorage.setItem('lachuoi-latest-tasks', JSON.stringify(data.status));
                    renderTable();
                } else if (data.webhook) {
                    // Update webhooks in localStorage
                    let savedWebhooks = [];
                    try {
                        const existing = localStorage.getItem('lachuoi-latest-webhooks');
                        if (existing) savedWebhooks = JSON.parse(existing);
                    } catch(e) {}
                    savedWebhooks.unshift(data.webhook);
                    if (savedWebhooks.length > 15) savedWebhooks = savedWebhooks.slice(0, 15);
                    localStorage.setItem('lachuoi-latest-webhooks', JSON.stringify(savedWebhooks));

                    if (!webhookTableBody) return;
                    
                    const urlParams = new URLSearchParams(window.location.search);
                    const currentPage = parseInt(urlParams.get('page') || '1');
                    if (currentPage !== 1) return;

                    const webhook = data.webhook;
                    const headers = JSON.parse(webhook.headers);
                    const fromAddress = headers['x-forwarded-for'] || '-';
                    
                    const row = document.createElement('tr');
                    row.id = `row-${webhook.id}`;
                    row.className = 'border-b border-gray-100 dark:border-slate-800 hover:bg-gray-50 dark:hover:bg-slate-800/50 transition-colors';
                    row.setAttribute('data-headers', webhook.headers.replace(/'/g, '&apos;'));
                    row.setAttribute('data-body', webhook.body.replace(/'/g, '&apos;'));
                    
                    row.innerHTML = `
                        <td class='px-4 py-3 align-middle text-xs font-mono text-gray-500 dark:text-slate-500'>${webhook.id}</td>
                        <td class='px-4 py-3 align-middle text-sm text-gray-600 dark:text-slate-400'>${webhook.created_at}</td>
                        <td class='px-4 py-3 align-middle'><span class='px-2 py-1 text-[10px] uppercase font-bold rounded bg-blue-50 text-blue-700 border border-blue-200 dark:bg-blue-900/20 dark:text-blue-400 dark:border-blue-800'>${webhook.method}</span></td>
                        <td class='px-4 py-3 align-middle text-sm font-mono text-gray-900 dark:text-slate-100'>${webhook.path}</td>
                        <td class='px-4 py-3 align-middle text-sm text-gray-600 dark:text-slate-400'>${fromAddress}</td>
                        <td class='px-4 py-3 align-middle'>
                            <div class='flex items-center gap-2'>
                                <button class='px-3 py-1 text-xs font-semibold text-blue-600 bg-blue-50 border border-blue-200 rounded-md hover:bg-blue-600 hover:text-white dark:bg-blue-900/20 dark:border-blue-800 dark:hover:bg-blue-600 transition-colors' onclick='showDetails(${webhook.id})'>Details</button>
                                <button class='px-3 py-1 text-xs font-semibold text-red-600 bg-red-50 border border-red-200 rounded-md hover:bg-red-600 hover:text-white dark:bg-red-900/20 dark:border-red-800 dark:hover:bg-red-600 transition-colors' onclick='deleteWebhook(${webhook.id})'>Delete</button>
                            </div>
                        </td>
                    `;
                    webhookTableBody.insertBefore(row, webhookTableBody.firstChild);
                    
                    while (webhookTableBody.children.length > 15) {
                        webhookTableBody.removeChild(webhookTableBody.lastChild);
                    }
                }
            } catch (e) {
                console.error("Error handling SSE message:", e);
            }
        };

        window.filterTask = function(taskId) {
            activeTaskIdFilter = taskId;
            const task = latestTasks.find(t => t.id === taskId);
            const taskName = task ? task.name : taskId;
            
            // Update UI to show filter is active
            const indicator = document.getElementById('status-indicator');
            if (indicator) {
                indicator.innerHTML = `Filtered: <span class="text-blue-400 font-bold">${taskName}</span>`;
            }

            const showAllBtn = document.getElementById('show-all-logs');
            if (showAllBtn) showAllBtn.classList.remove('hidden');

            // Hide/show existing logs
            const entries = document.querySelectorAll('.log-entry');
            entries.forEach(entry => {
                if (entry.getAttribute('data-task-id') === taskId) {
                    entry.classList.remove('hidden');
                } else {
                    entry.classList.add('hidden');
                }
            });
            if (logsDiv) logsDiv.scrollTop = logsDiv.scrollHeight;
        };

        window.clearFilter = function() {
            activeTaskIdFilter = null;
            const indicator = document.getElementById('status-indicator');
            if (indicator) {
                indicator.innerText = 'Connected';
                indicator.className = 'text-xs font-bold text-emerald-400';
            }

            const showAllBtn = document.getElementById('show-all-logs');
            if (showAllBtn) showAllBtn.classList.add('hidden');

            // Show all logs
            const entries = document.querySelectorAll('.log-entry');
            entries.forEach(entry => {
                entry.classList.remove('hidden');
            });
            if (logsDiv) logsDiv.scrollTop = logsDiv.scrollHeight;
        };

        window.toggleTask = async function(taskId, enabled) {
            try {
                const response = await fetch(`/tasks/${taskId}/toggle`, {
                    method: 'POST',
                    headers: { 'Content-Type': 'application/json' },
                    body: JSON.stringify({ enabled })
                });
                if (!response.ok) alert('Failed to update task status');
            } catch (error) {
                console.error('Error toggling task:', error);
                alert('Error toggling task status');
            }
        };

        window.runTask = async function(taskId) {
            try {
                const response = await fetch(`/tasks/${taskId}/run`, {
                    method: 'POST',
                });
                if (!response.ok) {
                    const errorText = await response.text();
                    alert(`Failed to run task: ${errorText}`);
                }
            } catch (error) {
                console.error('Error running task:', error);
                alert('Error running task');
            }
        };

        window.showTaskLogs = async function(logId) {
            try {
                const response = await fetch(`/logs/${logId}`);
                if (response.ok) {
                    const data = await response.json();
                    const logModal = document.getElementById('log-modal');
                    const logModalContent = document.getElementById('log-modal-content');
                    if (logModal && logModalContent) {
                        logModalContent.innerHTML = '';
                        data.logs.forEach(log => {
                            const entry = document.createElement('div');
                            entry.className = 'mb-1 flex gap-3 text-xs md:text-sm';
                            const isError = log.output.toLowerCase().includes('failed') || log.output.toLowerCase().includes('error');
                            const textColor = isError ? 'text-red-400 font-medium' : 'text-slate-300';
                            const timeStr = new Date(log.created_at).toLocaleTimeString();
                            entry.innerHTML = `<span class="text-slate-500 shrink-0 font-medium select-none">[${timeStr}]</span><span class="${textColor}">${log.output}</span>`;
                            logModalContent.appendChild(entry);
                        });
                        logModal.classList.add('active');
                    }
                } else {
                    alert('Failed to fetch logs for this run');
                }
            } catch (error) {
                console.error('Error fetching run logs:', error);
                alert('Error fetching run logs');
            }
        };

        window.closeLogModal = function() {
            const logModal = document.getElementById('log-modal');
            if (logModal) logModal.classList.remove('active');
        };

        eventSource.onopen = () => {
            if (statusIndicator) {
                statusIndicator.innerText = 'Connected';
                statusIndicator.className = 'text-xs font-bold text-emerald-400';
            }
        };

        eventSource.onerror = (e) => {
            if (statusIndicator) {
                statusIndicator.innerText = 'Disconnected - retrying...';
                statusIndicator.className = 'text-xs font-bold text-red-400';
            }
        };
    } catch (globalError) {
        console.error("Critical error in app.js:", globalError);
    }
})();
