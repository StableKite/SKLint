const vscode = require('vscode');
const cp = require('child_process');
const fs = require('fs');
const path = require('path');
const SOURCE = 'SKLint';
const DEBOUNCE_MS = 250;
let collection;
const timers = new Map();
const fixes = new Map();
const suppressionLines = new Map();
const activeDiagnostics = new Map();
let extensionRoot = "";
const RU_DIAGNOSTICS = {
    SK001: 'Пробелы и табы в конце строки запрещены',
    SK101: 'TODO-комментарии запрещены в строгом режиме',
    SK201: 'print разрешён только внутри блока if __name__ == "__main__"',
    SK211: 'Кириллические предложения в комментариях должны начинаться с заглавной буквы',
    SK212: 'Комментарии не должны заканчиваться точкой',
    SK301: 'Во вложенном классе между методами должна быть ровно одна пустая строка, а в остальных местах пустых строк быть не должно',
    SK302: 'Во вложенной функции не должно быть пустых строк в теле',
    SK303: 'В обычном классе между методами должно быть ровно две пустые строки',
    SK305: 'В теле функции или метода не должно быть больше одной пустой строки подряд',
    SK306: 'Самостоятельные публичные функции и классы должны разделяться ровно тремя пустыми строками',
    SK307: 'Перед блоком __main__ должно быть ровно три пустые строки',
    SK308: 'Внутри блока __main__ не должно быть больше одной пустой строки подряд',
    SK309: 'Файл не должен заканчиваться переносом строки',
    SK310: 'Приватный helper, используемый только следующим объектом, должен отделяться ровно двумя пустыми строками',
    SK311: 'В классе-заглушке между методами должна быть ровно одна пустая строка',
    SK312: 'Между top-level классами-заглушками должно быть ровно две пустые строки',
    SK313: 'Между ... и следующим докстрингом функции или метода-заглушки не должно быть пустой строки',
    SK314: 'В if TYPE_CHECKING между классами-заглушками без аргументов и методов должна быть ровно одна пустая строка',
    SK315: 'Между докстрингом функции или метода-заглушки и следующим ... не должно быть пустой строки',
    SK401: 'Оператор присваивания должен иметь пробелы с обеих сторон',
    SK403: 'Элементы многострочной конструкции в скобках должны идти по одному на строку с отступом',
    SK404: 'Висящие запятые запрещены вне import-блоков и одноэлементных tuple-литералов',
    SK502: 'Импорты должны использовать форму from-import, кроме sys.platform/sys.version_info import',
    SK503: 'Используйте sys.platform вместо os.name для проверок платформы',
    SK504: 'Используйте import sys и sys.platform вместо from sys import platform',
    SK505: 'Объявления должны располагаться выше первого использования',
    SK509: '__new__, __init__ и __post_init__ должны идти перед обычными методами класса именно в этом порядке',
    SK506: 'Блоки try, except и finally запрещены в горячем runtime-коде',
    SK507: 'raise разрешён только в lifecycle-методах и их приватных helper-методах',
    SK508: 'from __future__ import annotations запрещён',
    SK801: 'Промежуточную переменную с одним использованием нужно свернуть в строгом режиме',
    SK802: 'Ветки return нужно свернуть в тернарное выражение в строгом режиме',
    SK803: 'Цикл только с append нужно свернуть в list comprehension в строгом режиме',
    SK804: 'Модуль с публичными символами должен объявлять __all__ как tuple в строгом режиме',
    SK601: 'Строка докстринга длиннее 72 символов',
    SK602: 'Докстринг должен быть оформлен только в Google style',
    SK603: 'Последняя строка секции докстринга не должна заканчиваться точкой',
    SK604: 'Докстринг выглядит полностью английским и не содержит кириллицы',
    SK605: 'Описание должно быть сформулировано как процесс или состояние, а не как действие-глагол',
    SK606: 'В докстринге разрешены только секции Args, Attributes, Returns и Raises',
    SK607: 'Докстринг вложенного объекта не должен содержать пустые строки',
    SK608: 'Короткий докстринг вложенного объекта должен быть записан в одну строку',
    SK609: 'Константа Final должна иметь строку-докстринг с описанием',
    SK610: 'Короткий докстринг константы должен быть записан в одну строку',
    SK611: 'Короткий докстринг модуля должен быть записан в одну строку',
    SK612: 'У невложенных функций, методов и классов тройные кавычки должны быть на отдельных строках',
    SK613: 'После многострочного докстринга функции, метода или класса должна идти пустая строка',
    SK614: 'У функции, метода или класса должно быть описание помимо структурированных секций',
    SK615: 'Между описанием и структурированными секциями нужна пустая строка внутри докстринга',
    SK616: 'Описание не должно начинаться со слов Метод, Функция или Класс',
    SK617: 'Кириллические предложения в докстрингах должны начинаться с заглавной буквы',
    SK618: 'В конце строки докстринга запрещён whitespace, кроме markdown-переноса ровно двумя пробелами',
    SK619: 'Attributes у dataclass должен перечислять все поля, включая унаследованные',
    SK620: 'Типы в Attributes у dataclass должны совпадать с аннотациями полей',
    SK621: 'После докстринга модуля не должно быть пустых строк перед кодом',
    SK622: 'Между Args, Returns и Raises не должно быть пустых строк',
    SK623: 'Секции функций и методов должны идти в порядке Args, Returns, Raises',
    SK624: 'Длинное описание аргумента, атрибута или исключения должно начинаться со следующей строки с отступом',
    SK701: 'Атрибут self вводится вне __init__ и не объявлен в классе',
    SK702: 'Динамический атрибут объекта не объявлен в его классе',
    SK900: 'Подавление SKLint больше не используется',
};
const RU_FIXES = {
    SK001: 'Удалить завершающий whitespace',
    SK211: 'Сделать первую кириллическую букву заглавной',
    SK212: 'Удалить точку в конце комментария',
    SK301: 'Нормализовать пустые строки во вложенном классе',
    SK302: 'Удалить пустую строку из вложенной функции',
    SK303: 'Нормализовать пустые строки между методами класса',
    SK305: 'Сократить пустые строки в теле функции или метода',
    SK306: 'Нормализовать пустые строки между самостоятельными объектами',
    SK307: 'Нормализовать пустые строки перед __main__',
    SK308: 'Сократить пустые строки внутри __main__',
    SK309: 'Удалить финальный перенос строки',
    SK310: 'Нормализовать пустые строки после приватного helper',
    SK311: 'Нормализовать пустые строки между методами заглушки',
    SK312: 'Нормализовать пустые строки между классами-заглушками',
    SK313: 'Убрать пустую строку между ... и докстрингом',
    SK314: 'Нормализовать пустые строки между TYPE_CHECKING классами-заглушками',
    SK315: 'Убрать пустую строку между докстрингом и ...',
    SK401: 'Нормализовать пробелы вокруг =',
    SK403: 'Разнести элементы многострочной конструкции по строкам',
    SK404: 'Удалить висящую запятую',
    SK502: 'Переписать import в форму from-import',
    SK503: 'Заменить os.name на sys.platform',
    SK504: 'Переписать platform import на sys.platform',
    SK508: 'Удалить from __future__ import annotations',
    SK801: 'Свернуть промежуточную переменную',
    SK802: 'Свернуть return в тернарное выражение',
    SK803: 'Свернуть цикл append в list comprehension',
    SK804: 'Создать __all__ tuple',
    SK603: 'Удалить точку в конце секции',
    SK608: 'Записать вложенный докстринг в одну строку',
    SK609: 'Создать заготовку докстринга константы',
    SK610: 'Записать докстринг константы в одну строку',
    SK611: 'Записать докстринг модуля в одну строку',
    SK612: 'Перенести кавычки докстринга на отдельные строки',
    SK613: 'Добавить пустую строку после докстринга',
    SK614: 'Добавить заготовку описания докстринга',
    SK615: 'Добавить пустую строку перед секциями',
    SK616: 'Удалить лишнее слово в начале описания',
    SK617: 'Сделать первую букву предложения заглавной',
    SK618: 'Удалить завершающий whitespace в докстринге',
    SK621: 'Удалить пустую строку после докстринга модуля',
    SK622: 'Удалить пустую строку между секциями',
    SK624: 'Перенести длинное описание элемента на следующую строку',
};
function activate(context) {
    extensionRoot = context.extensionPath || '';
    collection = vscode.languages.createDiagnosticCollection('sklint');
    context.subscriptions.push(collection);
    context.subscriptions.push(vscode.workspace.onDidOpenTextDocument((doc) => scheduleLint(doc)));
    context.subscriptions.push(vscode.workspace.onDidSaveTextDocument((doc) => lintDocument(doc)));
    context.subscriptions.push(vscode.workspace.onDidCloseTextDocument((doc) => {
        collection.delete(doc.uri);
        activeDiagnostics.delete(doc.uri.toString());
    }));
    context.subscriptions.push(vscode.workspace.onDidChangeTextDocument((event) => {
        const runMode = vscode.workspace.getConfiguration('sklint', event.document.uri).get('run', 'onType');
        if (runMode === 'onType') {
            scheduleLint(event.document);
        }
    }));
    context.subscriptions.push(vscode.languages.onDidChangeDiagnostics((event) => {
        for (const uri of event.uris || []) {
            const doc = vscode.workspace.textDocuments.find((item) => item.uri.toString() === uri.toString());
            if (doc && isPythonFile(doc)) {
                scheduleLint(doc);
            }
        }
    }));
    context.subscriptions.push(vscode.languages.registerCodeActionsProvider({ language: 'python', scheme: 'file' }, new SKLintCodeActionProvider(), { providedCodeActionKinds: [vscode.CodeActionKind.QuickFix] }));
    context.subscriptions.push(vscode.languages.registerDocumentFormattingEditProvider({ language: 'python', scheme: 'file' }, new SKLintFormattingProvider()));
    context.subscriptions.push(vscode.languages.registerHoverProvider({ language: 'python', scheme: 'file' }, new SKLintHoverProvider()));
    context.subscriptions.push(vscode.commands.registerCommand('sklint.fixAll', async () => {
        const editor = vscode.window.activeTextEditor;
        if (!editor || !isPythonFile(editor.document)) {
            vscode.window.showWarningMessage(l10n('Open a .py or .pyi file', 'Откройте .py или .pyi файл'));
            return;
        }
        const edits = await formatEdits(editor.document);
        if (!edits || edits.length === 0) {
            vscode.window.showInformationMessage(l10n('No safe SKLint fixes available', 'Автоисправлений SKLint нет'));
            return;
        }
        const workspaceEdit = new vscode.WorkspaceEdit();
        for (const edit of edits) {
            workspaceEdit.replace(editor.document.uri, edit.range, edit.newText);
        }
        await vscode.workspace.applyEdit(workspaceEdit);
        await editor.document.save();
        lintDocument(editor.document);
    }));
    for (const doc of vscode.workspace.textDocuments) {
        scheduleLint(doc);
    }
}
function deactivate() {
    for (const timer of timers.values()) {
        clearTimeout(timer);
    }
    timers.clear();
    fixes.clear();
    suppressionLines.clear();
    activeDiagnostics.clear();
}
function scheduleLint(document) {
    if (!isPythonFile(document)) {
        return;
    }
    const key = document.uri.toString();
    const old = timers.get(key);
    if (old) {
        clearTimeout(old);
    }
    timers.set(key, setTimeout(() => {
        timers.delete(key);
        lintDocument(document);
    }, DEBOUNCE_MS));
}
function lintDocument(document) {
    if (!isPythonFile(document)) {
        return;
    }
    const command = resolveSklintCommand(document.uri);
    const args = [...command.args, 'check', '--format', 'json', '--stdin-filename', document.fileName, ...vscodeConfigArgs(document.uri), '-'];
    cp.execFile(command.executable, args, {
        cwd: workspaceFolderFor(document.uri),
        timeout: 10000,
        maxBuffer: 4 * 1024 * 1024,
    }, async (error, stdout, stderr) => {
        if (error && error.code !== 1) {
            const range = new vscode.Range(0, 0, 0, 1);
            const diagnostic = new vscode.Diagnostic(range, `${SOURCE} ${l10n('failed to run', 'не запустился')}: ${stderr || error.message}`, vscode.DiagnosticSeverity.Warning);
            diagnostic.source = SOURCE;
            collection.set(document.uri, [diagnostic]);
            activeDiagnostics.set(document.uri.toString(), [diagnostic]);
            return;
        }
        let payload;
        try {
            payload = JSON.parse(stdout || '{"diagnostics":[]}');
        }
        catch (parseError) {
            const diagnostic = new vscode.Diagnostic(new vscode.Range(0, 0, 0, 1), `${SOURCE} ${l10n('returned invalid JSON', 'вернул некорректный JSON')}: ${parseError.message}`, vscode.DiagnosticSeverity.Warning);
            diagnostic.source = SOURCE;
            collection.set(document.uri, [diagnostic]);
            activeDiagnostics.set(document.uri.toString(), [diagnostic]);
            return;
        }
        clearDocumentCaches(document.uri);
        const pyrightDiagnostics = pyrightLikeDiagnostics(document.uri);
        const semanticRanges = await semanticTokenRanges(document);
        const diagnostics = [];
        for (const item of payload.diagnostics || []) {
            const startLine = Math.max(0, Number(item.line || 1) - 1);
            const startColumn = Math.max(0, Number(item.column || 1) - 1);
            const endLine = Math.max(startLine, Number(item.end_line || item.line || 1) - 1);
            const endColumn = Math.max(startColumn + 1, Number(item.end_column || item.column || 1) - 1);
            const range = new vscode.Range(startLine, startColumn, endLine, endColumn);
            if (!shouldKeepDiagnostic(document, item, range, pyrightDiagnostics, semanticRanges)) {
                continue;
            }
            const diagnostic = new vscode.Diagnostic(range, localizedDiagnosticMessage(item), diagnosticSeverity(item));
            diagnostic.code = String(item.code || '').toUpperCase();
            diagnostic.source = SOURCE;
            diagnostics.push(diagnostic);
            const key = diagnosticKey(document.uri, diagnostic);
            if (item.fix) {
                fixes.set(key, { ...item.fix, message: localizedFixMessage(item.code, item.fix.message) });
            }
            if (item.suppression_line) {
                suppressionLines.set(key, Math.max(0, Number(item.suppression_line) - 1));
            }
        }
        collection.set(document.uri, diagnostics);
        activeDiagnostics.set(document.uri.toString(), diagnostics);
    }).stdin.end(document.getText());
}
function clearDocumentCaches(uri) {
    const documentKey = `${uri.toString()}|`;
    for (const key of Array.from(fixes.keys())) {
        if (String(key).startsWith(documentKey)) {
            fixes.delete(String(key));
        }
    }
    for (const key of Array.from(suppressionLines.keys())) {
        if (String(key).startsWith(documentKey)) {
            suppressionLines.delete(String(key));
        }
    }
}
function shouldKeepDiagnostic(document, item, range, pyrightDiagnostics, semanticRanges) {
    const code = String(item.code || '').toUpperCase();
    if (code !== 'SK701' && code !== 'SK702') {
        return true;
    }
    const config = vscode.workspace.getConfiguration('sklint', document.uri);
    if (config.get('pyrightAware.filterDiagnostics', true) && pyrightDiagnostics.some((diag) => rangesOverlap(range, diag.range))) {
        return false;
    }
    if (config.get('pyrightAware.requireMissingSemanticToken', true) && semanticRanges.length > 0) {
        return !semanticRanges.some((semantic) => rangesOverlap(range, semantic.range));
    }
    return true;
}
function pyrightLikeDiagnostics(uri) {
    return (vscode.languages.getDiagnostics(uri) || []).filter((diag) => {
        const source = String(diag.source || '').toLowerCase();
        return diag.source !== SOURCE && (source.includes('pyright') || source.includes('pylance'));
    });
}
async function semanticTokenRanges(document) {
    const config = vscode.workspace.getConfiguration('sklint', document.uri);
    if (!config.get('pyrightAware.requireMissingSemanticToken', true)) {
        return [];
    }
    try {
        const legend = await vscode.commands.executeCommand('vscode.provideDocumentSemanticTokensLegend', document.uri);
        const tokens = await vscode.commands.executeCommand('vscode.provideDocumentSemanticTokens', document.uri);
        if (!legend || !tokens || !tokens.data) {
            return [];
        }
        const data = Array.from(tokens.data);
        const ranges = [];
        let line = 0;
        let character = 0;
        for (let i = 0; i + 4 < data.length; i += 5) {
            const deltaLine = Number(data[i]);
            const deltaStart = Number(data[i + 1]);
            const length = Number(data[i + 2]);
            line += deltaLine;
            character = deltaLine === 0 ? character + deltaStart : deltaStart;
            ranges.push({ range: new vscode.Range(line, character, line, character + length) });
        }
        return ranges;
    }
    catch (_err) {
        return [];
    }
}
function rangesOverlap(a, b) {
    return a.start.isBefore(b.end) && b.start.isBefore(a.end);
}
class SKLintHoverProvider {
    provideHover(document, position) {
        const codeRange = suppressionCodeAtPosition(document, position);
        if (!codeRange) {
            return undefined;
        }
        const markdown = ruleHelpHoverMarkdown(codeRange.code);
        if (!markdown) {
            return undefined;
        }
        return new vscode.Hover(markdown, codeRange.range);
    }
}
function suppressionCodeAtPosition(document, position) {
    const line = document.lineAt(position.line).text;
    const hash = line.indexOf('#');
    if (hash < 0 || position.character < hash) {
        return undefined;
    }
    const comment = line.slice(hash + 1);
    const lower = comment.toLowerCase();
    if (!lower.includes('noqa') && !lower.includes('sklint:')) {
        return undefined;
    }
    const pattern = /\bSK\d{3}\b/gi;
    let match;
    while ((match = pattern.exec(line)) !== null) {
        const start = match.index;
        const end = start + match[0].length;
        if (start < hash) {
            continue;
        }
        const range = new vscode.Range(position.line, start, position.line, end);
        if (range.contains(position) || position.character === end) {
            return { code: match[0].toUpperCase(), range };
        }
    }
    return undefined;
}
function lineDiagnosticsForCodeActions(document, range, context) {
    const seen = new Set();
    const result = [];
    const addDiagnostic = (diagnostic) => {
        if (!diagnostic || diagnostic.source !== SOURCE) {
            return;
        }
        const key = diagnosticKey(document.uri, diagnostic);
        if (seen.has(key)) {
            return;
        }
        seen.add(key);
        result.push(diagnostic);
    };
    for (const diagnostic of context.diagnostics || []) {
        addDiagnostic(diagnostic);
    }
    // VSCode normally passes all diagnostics under the cursor in context.diagnostics.
    // Still, some versions/providers can pass only the first one. Recover only the
    // diagnostics that actually intersect the requested code-action range; do not
    // pull every SKLint diagnostic from the same line, or the menu becomes noisy.
    for (const diagnostic of activeDiagnostics.get(document.uri.toString()) || []) {
        if (diagnostic.source !== SOURCE || !diagnosticIntersectsRequest(diagnostic, range)) {
            continue;
        }
        addDiagnostic(diagnostic);
    }
    return result.sort(compareDiagnostics);
}
function diagnosticIntersectsRequest(diagnostic, range) {
    if (!diagnostic || !range) {
        return false;
    }
    if (range.isEmpty) {
        return diagnostic.range.contains(range.start);
    }
    return rangesOverlap(diagnostic.range, range) || diagnostic.range.contains(range.start) || diagnostic.range.contains(range.end);
}
function compareDiagnostics(a, b) {
    const byStart = a.range.start.character - b.range.start.character;
    if (byStart !== 0) {
        return byStart;
    }
    return diagnosticCodeValue(a).localeCompare(diagnosticCodeValue(b));
}
function ruleHelpHoverMarkdown(code, message) {
    const markdown = new vscode.MarkdownString('', false);
    markdown.isTrusted = false;
    markdown.supportHtml = false;
    markdown.appendMarkdown(`### ${SOURCE} ${code}

`);
    if (message) {
        markdown.appendMarkdown(`${escapeMarkdownText(stripTrailingSentencePeriod(message))}

`);
    }
    const help = ruleHelpMarkdown(code);
    if (help) {
        markdown.appendMarkdown(`${help.trim()}
`);
    }
    return markdown;
}
function ruleHelpMarkdown(code) {
    const upper = String(code || '').toUpperCase();
    const docsPath = resolveDocsPath();
    if (!docsPath || !fs.existsSync(docsPath)) {
        return undefined;
    }
    const content = fs.readFileSync(docsPath, 'utf8');
    const section = extractRuleSection(content, upper);
    if (!section) {
        return undefined;
    }
    return demoteMarkdownHeadings(section);
}
function demoteMarkdownHeadings(markdown) {
    return markdown
        .split(/\r?\n/)
        .map((line) => line.startsWith('#') ? `##${line}` : line)
        .join('\n');
}
function escapeMarkdownText(text) {
    return text.replace(/[\\`*_{}[\]()#+\-.!|>]/g, '\\$&');
}
function sklintActionTitle(code, text) {
    const cleaned = stripTrailingSentencePeriod(String(text || code).trim());
    return `SKLint (${code}): ${cleaned}`;
}
function localizedSuppressTitle(code) {
    return sklintActionTitle(code, l10n(`Suppress for SKLint`, `Подавить для SKLint`));
}
class SKLintCodeActionProvider {
    provideCodeActions(document, range, context) {
        const actions = [];
        let addedFixAll = false;
        const addedFixes = new Set();
        const addedSuppressions = new Set();
        const diagnostics = lineDiagnosticsForCodeActions(document, range, context);
        for (const diagnostic of diagnostics) {
            if (diagnostic.source !== SOURCE || !diagnostic.code) {
                continue;
            }
            const key = diagnosticKey(document.uri, diagnostic);
            const code = diagnosticCodeValue(diagnostic);
            const fix = fixes.get(key);
            if (fix) {
                const fixTitle = sklintActionTitle(code, fix.message || l10n('Fix', 'Исправить'));
                // VSCode can ask for code actions for an overlapping diagnostic range;
                // for large docstring diagnostics this may include several equal SKLint
                // fixes from the same block. Show only one action per visible title so
                // the quick-fix menu stays compact, like Ruff's menu.
                const fixKey = fixTitle;
                if (!addedFixes.has(fixKey)) {
                    addedFixes.add(fixKey);
                    const action = new vscode.CodeAction(fixTitle, vscode.CodeActionKind.QuickFix);
                    action.diagnostics = [diagnostic];
                    action.isPreferred = true;
                    action.edit = new vscode.WorkspaceEdit();
                    action.edit.replace(document.uri, new vscode.Range(Math.max(0, fix.start_line - 1), Math.max(0, fix.start_column - 1), Math.max(0, fix.end_line - 1), Math.max(0, fix.end_column - 1)), fix.replacement || '');
                    actions.push(action);
                }
            }
            if (!addedFixAll) {
                const title = l10n('SKLint: Fix all safe diagnostics', 'SKLint: исправить все безопасные предупреждения');
                const fixAll = new vscode.CodeAction(title, vscode.CodeActionKind.QuickFix);
                fixAll.command = { command: 'sklint.fixAll', title };
                actions.push(fixAll);
                addedFixAll = true;
            }
            const suppressLine = suppressionLines.get(key) ?? diagnostic.range.start.line;
            const suppressKey = `${code}|${suppressLine}`;
            if (!addedSuppressions.has(suppressKey)) {
                addedSuppressions.add(suppressKey);
                const suppress = new vscode.CodeAction(localizedSuppressTitle(code), vscode.CodeActionKind.QuickFix);
                suppress.diagnostics = diagnostics
                    .filter((item) => diagnosticCodeValue(item) === code)
                    .filter((item) => (suppressionLines.get(diagnosticKey(document.uri, item)) ?? item.range.start.line) === suppressLine);
                suppress.edit = new vscode.WorkspaceEdit();
                const edit = buildNoqaEdit(document, suppressLine, code);
                if (edit) {
                    if (edit.replaceRange) {
                        suppress.edit.replace(document.uri, edit.replaceRange, edit.text);
                    }
                    else {
                        suppress.edit.insert(document.uri, edit.position, edit.text);
                    }
                    actions.push(suppress);
                }
            }
        }
        return dedupeActionsByTitle(actions);
    }
}
function dedupeActionsByTitle(actions) {
    const seen = new Set();
    const result = [];
    for (const action of actions) {
        const title = String(action.title || '');
        if (seen.has(title)) {
            continue;
        }
        seen.add(title);
        result.push(action);
    }
    return result;
}
class SKLintFormattingProvider {
    async provideDocumentFormattingEdits(document) {
        const enabled = vscode.workspace.getConfiguration('sklint', document.uri).get('formatting.enabled', false);
        if (!enabled || !isPythonFile(document)) {
            return [];
        }
        return formatEdits(document);
    }
}
async function formatEdits(document) {
    const command = resolveSklintCommand(document.uri);
    const args = [...command.args, 'format', '--stdin-filename', document.fileName, ...vscodeConfigArgs(document.uri), '-'];
    const formatted = await execFileWithStdin(command.executable, args, document.getText(), workspaceFolderFor(document.uri));
    if (formatted === document.getText()) {
        return [];
    }
    const fullRange = new vscode.Range(document.positionAt(0), document.positionAt(document.getText().length));
    return [new vscode.TextEdit(fullRange, formatted)];
}
function execFileWithStdin(executable, args, input, cwd) {
    return new Promise((resolve, reject) => {
        const child = cp.execFile(executable, args, { cwd, timeout: 10000, maxBuffer: 8 * 1024 * 1024 }, (error, stdout, stderr) => {
            if (error) {
                reject(new Error(stderr || error.message));
                return;
            }
            resolve(stdout);
        });
        child.stdin.end(input);
    });
}
function buildNoqaEdit(document, lineNumber, code) {
    const line = document.lineAt(lineNumber);
    const text = line.text;
    const noqa = /#\s*noqa(?::\s*([^#]*))?/i.exec(text);
    if (noqa && noqa.index !== undefined) {
        const whole = noqa[0];
        const codesText = noqa[1];
        if (codesText !== undefined) {
            const codes = codesText.split(',').map((part) => part.trim()).filter(Boolean);
            if (codes.map((c) => c.toUpperCase()).includes(code.toUpperCase())) {
                return undefined;
            }
            const prefixEnd = noqa.index + whole.length;
            const replacement = whole.replace(codesText, [...codes, code].join(', '));
            return {
                replaceRange: new vscode.Range(lineNumber, noqa.index, lineNumber, prefixEnd),
                text: replacement,
            };
        }
        return {
            position: new vscode.Position(lineNumber, text.length),
            text: `  # sklint: ignore ${code}`,
        };
    }
    return {
        position: new vscode.Position(lineNumber, text.length),
        text: `  # noqa: ${code}`,
    };
}
function extractRuleSection(markdown, code) {
    const lines = markdown.split(/\r?\n/);
    const start = lines.findIndex((line) => line.startsWith(`## ${code} `) || line.startsWith(`## ${code} —`));
    if (start < 0) {
        return undefined;
    }
    let end = lines.length;
    for (let i = start + 1; i < lines.length; i += 1) {
        if (lines[i].startsWith('## ')) {
            end = i;
            break;
        }
    }
    return lines.slice(start, end).join('\n');
}
function resolveDocsPath() {
    const folders = vscode.workspace.workspaceFolders || [];
    for (const folder of folders) {
        const candidate = path.join(folder.uri.fsPath, 'docs', 'rules.ru.md');
        if (fs.existsSync(candidate)) {
            return candidate;
        }
    }
    const candidates = [
        extensionRoot ? path.join(extensionRoot, 'docs', 'rules.ru.md') : '',
        __dirname ? path.resolve(__dirname, '..', 'docs', 'rules.ru.md') : '',
        __dirname ? path.resolve(__dirname, '..', '..', 'docs', 'rules.ru.md') : '',
    ];
    for (const candidate of candidates) {
        if (candidate && fs.existsSync(candidate)) {
            return candidate;
        }
    }
    return undefined;
}
function vscodeConfigArgs(uri) {
    const config = vscode.workspace.getConfiguration('sklint', uri);
    const args = ['--vscode-strict', String(config.get('strict', false))];
    const select = config.get('select', []);
    const ignore = config.get('ignore', []);
    if (Array.isArray(select) && select.length > 0) {
        args.push('--vscode-select', select.join(','));
    }
    if (Array.isArray(ignore) && ignore.length > 0) {
        args.push('--vscode-ignore', ignore.join(','));
    }
    return args;
}
function isPythonFile(document) {
    if (!document || document.uri.scheme !== 'file') {
        return false;
    }
    return document.fileName.endsWith('.py') || document.fileName.endsWith('.pyi');
}
function resolveSklintCommand(uri) {
    const executable = resolveExecutable(uri);
    if (executable) {
        return { executable, args: [] };
    }
    return { executable: 'sklint', args: [] };
}
function resolveExecutable(uri) {
    const config = vscode.workspace.getConfiguration('sklint', uri);
    const configuredValues = [
        String(config.get('executablePath', '') || '').trim(),
        String(config.get('path', '') || '').trim(),
    ];
    for (const configured of configuredValues) {
        const configuredPath = resolveConfiguredExecutable(configured, uri);
        if (configuredPath) {
            return configuredPath;
        }
    }
    const exe = executableName();
    const folder = uri ? vscode.workspace.getWorkspaceFolder(uri) : undefined;
    const workspace = folder ? folder.uri.fsPath : undefined;
    const candidates = [];
    if (workspace) {
        candidates.push(path.join(workspace, 'target', 'release', exe), path.join(workspace, 'target', 'debug', exe), path.join(workspace, '.venv', process.platform === 'win32' ? 'Scripts' : 'bin', exe), path.join(workspace, 'venv', process.platform === 'win32' ? 'Scripts' : 'bin', exe));
    }
    for (const candidate of bundledExecutableCandidates()) {
        candidates.push(candidate);
    }
    for (const candidate of candidates) {
        if (isExecutableFile(candidate)) {
            return candidate;
        }
    }
    return 'sklint';
}
function resolveConfiguredExecutable(configured, uri) {
    if (!configured) {
        return undefined;
    }
    const expanded = expandHome(configured);
    const candidates = [];
    if (path.isAbsolute(expanded)) {
        candidates.push(expanded);
    }
    else {
        const folder = uri ? vscode.workspace.getWorkspaceFolder(uri) : undefined;
        if (folder) {
            candidates.push(path.join(folder.uri.fsPath, expanded));
        }
        candidates.push(expanded);
    }
    for (const candidate of candidates) {
        if (isExecutableFile(candidate)) {
            return candidate;
        }
    }
    if (!looksLikePath(expanded)) {
        return expanded;
    }
    return undefined;
}
function bundledExecutableCandidates() {
    if (!extensionRoot) {
        return [];
    }
    const exe = executableName();
    const tags = platformTags();
    const candidates = [];
    for (const tag of tags) {
        candidates.push(path.join(extensionRoot, 'bin', tag, exe));
    }
    candidates.push(path.join(extensionRoot, 'bin', exe));
    return candidates;
}
function platformTags() {
    const arch = process.arch === 'x64' ? 'x64' : process.arch;
    const tags = [`${process.platform}-${arch}`];
    if (process.platform === 'win32') {
        tags.push('windows-x64', 'win32');
    }
    else if (process.platform === 'linux') {
        tags.push('linux-x64', 'linux');
    }
    else if (process.platform === 'darwin') {
        tags.push('darwin-x64', 'darwin');
    }
    return Array.from(new Set(tags));
}
function executableName() {
    return process.platform === 'win32' ? 'sklint.exe' : 'sklint';
}
function isExecutableFile(candidate) {
    try {
        return Boolean(candidate) && fs.existsSync(candidate) && fs.statSync(candidate).isFile();
    }
    catch (_error) {
        return false;
    }
}
function looksLikePath(value) {
    return path.isAbsolute(value) || value.includes('/') || value.includes('\\');
}
function expandHome(value) {
    if (value === '~') {
        return process.env.HOME || process.env.USERPROFILE || value;
    }
    if (value.startsWith('~/') || value.startsWith('~\\')) {
        const home = process.env.HOME || process.env.USERPROFILE;
        if (home) {
            return path.join(home, value.slice(2));
        }
    }
    return value;
}
function workspaceFolderFor(uri) {
    if (uri) {
        const folder = vscode.workspace.getWorkspaceFolder(uri);
        return folder ? folder.uri.fsPath : path.dirname(uri.fsPath);
    }
    const folders = vscode.workspace.workspaceFolders || [];
    return folders.length > 0 ? folders[0].uri.fsPath : process.cwd();
}
function diagnosticCodeValue(diagnostic) {
    const code = diagnostic?.code;
    if (code && typeof code === 'object' && 'value' in code) {
        return String(code.value || '').toUpperCase();
    }
    return String(code || '').toUpperCase();
}
function diagnosticKey(uri, diagnostic) {
    return `${uri.toString()}|${diagnosticCodeValue(diagnostic)}|${diagnostic.range.start.line}|${diagnostic.range.start.character}`;
}
function localizedDiagnosticMessage(item) {
    const code = String(item.code || '').toUpperCase();
    const fallback = stripTrailingSentencePeriod(String(item.message || code));
    return isRussianVscode() ? (RU_DIAGNOSTICS[code] || fallback) : fallback;
}
function diagnosticSeverity(item) {
    const level = String(item.level || '').toLowerCase();
    if (level === 'information' || level === 'info') {
        return vscode.DiagnosticSeverity.Information;
    }
    if (level === 'hint') {
        return vscode.DiagnosticSeverity.Hint;
    }
    if (level === 'error') {
        return vscode.DiagnosticSeverity.Error;
    }
    return vscode.DiagnosticSeverity.Warning;
}
function localizedFixMessage(code, fallback) {
    const upper = String(code || '').toUpperCase();
    const text = stripTrailingSentencePeriod(String(fallback || `Fix ${upper}`));
    return isRussianVscode() ? (RU_FIXES[upper] || text) : text;
}
function l10n(en, ru) {
    return isRussianVscode() ? ru : en;
}
function isRussianVscode() {
    return String(vscode.env.language || '').toLowerCase().startsWith('ru');
}
function stripTrailingSentencePeriod(text) {
    return text.replace(/[.。]+$/u, '');
}
module.exports = { activate, deactivate };
