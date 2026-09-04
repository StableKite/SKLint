# SKLint: документация предупреждений

Все правила работают для `.py` и `.pyi`. Для правил докстрингов `SK6xx` подавление строки ищется на последней строке соответствующего докстринга, как в pydoclint:

```python
def f():
    """
    Описание
    """  # noqa: SK601
```

## Автоисправления и форматтер

CLI-команда `sklint format` и VSCode formatter используют один и тот же механизм безопасных исправлений из ядра. Автофикс применяется только там, где изменение можно выполнить без самостоятельного придумывания смысла кода: удаление whitespace, нормализация пустых строк, перенос/переоформление кавычек, добавление технических пустых строк, перестановка структурированных секций, перестроение `Attributes` по фактическим dataclass-полям, исправление типов атрибутов по аннотациям и генерация `TODO`-заготовок там, где без текста человека не обойтись.

В VSCode форматтер по умолчанию выключен и включается настройкой `sklint.formatting.enabled = true`. Команда `SKLint: Fix All Safe Diagnostics` использует тот же встроенный форматтер.

Расширенная справка по правилу показывается в hover при наведении на код `SKxxx` внутри подавления `# noqa: SKxxx` или `# sklint: ignore SKxxx`.

## SK001 — TrailingWhitespace

**Уровень:** информационный.  
**Autofix:** есть.

Сообщает о пробелах и табах в конце строки. В VSCode отображается как информационная диагностика.

## SK101 — TodoComment

**Уровень:** strict-only.  
**Autofix:** нет.

В строгом режиме запрещает `TODO` в комментариях.

## SK201 — PrintStatement

**Уровень:** обычный.  
**Autofix:** нет.

Запрещает вызовы `print(...)` вне блока `if __name__ == "__main__":`.
Правило является проектным аналогом Ruff `T201`: вывод CLI допускается только в явной точке входа, а production/runtime-код должен использовать другой механизм вывода или логирования. Подавление SKLint выполняется только внутренним селектором `# noqa: SK201`. Ruff-селектор `T201` остаётся собственностью Ruff и SKLint его не интерпретирует.

## SK211 — CommentCyrillicSentenceCapitalized

**Уровень:** обычный.  
**Autofix:** есть.

Кириллические предложения в комментариях должны начинаться с заглавной буквы. Точка после цифры и точка внутри известных сокращений не считается концом предложения, поэтому конструкции вроде `Python 3.14 работает` и `и т.д. работает` не требуют заглавной буквы после такой точки. Директивы `noqa`, `type: ignore`, `pyright`, `ruff`, `pylint`, `sklint` и похожие служебные комментарии игнорируются.

## SK212 — CommentTrailingPeriod

**Уровень:** обычный.  
**Autofix:** есть.

Комментарий не должен заканчиваться точкой. Точки внутри комментария между предложениями разрешены, но финальная точка убирается. Точка после цифры и точка внутри известных сокращений не считается завершающей точкой предложения, поэтому комментарий может заканчиваться конструкциями вроде `Python 3.14`, `т.д.`, `т.п.`.


## SK301 — NestedClassBlankLines

**Уровень:** обычный.  
**Autofix:** есть.

Во вложенных классах между методами должна быть ровно одна пустая строка. В остальных местах тела вложенного класса пустые строки запрещены.

## SK302 — NestedFunctionBlankLines

**Уровень:** обычный.  
**Autofix:** есть.

Во вложенных функциях не должно быть пустых строк в теле вообще.

## SK303 — RegularClassMethodBlankLines

**Уровень:** обычный.  
**Autofix:** есть.

В обычных, не вложенных и не stub-классах между методами должно быть ровно две пустые строки.

## SK305 — FunctionBodyConsecutiveBlankLines

**Уровень:** обычный.  
**Autofix:** есть.

Внутри функций и методов не должно быть больше одной пустой строки подряд.

## SK306 — StandaloneTopLevelObjectBlankLines

**Уровень:** обычный.  
**Autofix:** есть.

Самостоятельные функции и классы верхнего уровня должны отделяться друг от друга ровно тремя пустыми строками, даже если имя начинается с `_`. Приватные helper-объекты, используемые только следующим объектом, обрабатываются правилом `SK310`; приватные entry/demo-функции, вызываемые из следующего блока `if __name__ == "__main__":`, считаются самостоятельными объектами.

## SK307 — MainBlockBlankLinesBefore

**Уровень:** обычный.  
**Autofix:** есть.

Между блоком `if __name__ == "__main__":` и тем, что идёт до него, должно быть ровно три пустые строки.

## SK308 — MainBlockConsecutiveBlankLines

**Уровень:** обычный.  
**Autofix:** есть.

Внутри блока `if __name__ == "__main__":` не должно быть больше одной пустой строки подряд.

## SK309 — FinalNewline

**Уровень:** обычный.  
**Autofix:** есть.

Файл не должен заканчиваться переносом строки. Форматтер удаляет финальный `
` и лишние финальные пустые строки.

## SK310 — PrivateHelperBlankLines

**Уровень:** обычный.  
**Autofix:** есть.

Если приватная функция или приватный класс верхнего уровня используется только в следующей функции или классе ниже, между helper-объектом и использующим его объектом должно быть ровно две пустые строки.

## SK311 — StubClassMethodBlankLines

**Уровень:** обычный.  
**Autofix:** есть.

Для классов-заглушек, у которых методы не содержат реализации (`...`, `pass`, `raise NotImplementedError`) или класс вообще не содержит методов и атрибутов, между методами должна быть ровно одна пустая строка.

## SK312 — StubClassBlankLines

**Уровень:** обычный.  
**Autofix:** есть.

Между top-level классами-заглушками должно быть ровно две пустые строки.

## SK313 — StubEllipsisDocstringBlankLines

**Уровень:** обычный.  
**Autofix:** есть.

В функциях и методах-заглушках между строкой `...` и следующим за ней строковым описанием не должно быть пустой строки.

## SK314 — TypeCheckingStubClassBlankLines

**Уровень:** обычный.  
**Autofix:** есть.

Внутри блока `if TYPE_CHECKING:` между классами-заглушками без аргументов наследования и без методов должна быть ровно одна пустая строка.

## SK315 — StubDocstringEllipsisBlankLines

**Уровень:** обычный.  
**Autofix:** есть.

В функциях и методах-заглушках между закрывающей строкой докстринга и следующей за ним строкой `...` не должно быть пустой строки. `SK613` такие заглушки не проверяет.

## SK601 — DocstringLineTooLong

**Уровень:** обычный.  
**Autofix:** частичный: длинные строки описания переносятся по ближайшему пробелу; строки элементов `Args`/`Attributes`/`Raises` исправляет `SK624`.

Строка докстринга не должна превышать 72 символа от начала строки, включая тройные кавычки, но исключая пробелы в конце строки и перенос строки. В VSCode подчёркивается только хвост строки после 72-го символа.

## SK602 — DocstringConfiguredStyle

**Уровень:** обычный.  
**Autofix:** структурный formatter приводит создаваемые/добавляемые секции к настроенному стилю; произвольный существующий докстринг целиком автоматически не переписывается.

Стиль докстринга должен совпадать с effective style SKLint. По умолчанию formatter и продукт используют Google style; через `formatter-docstring-style` / `sklint.formatting.docstringStyle` либо pydoclint `style` можно выбрать `google`, `numpy` или `sphinx`. `SK602`, pydoclint-порт и formatter используют один и тот же effective style, поэтому они не должны требовать взаимоисключающие форматы.

## SK603 — DocstringSectionTrailingPeriod

**Уровень:** обычный.  
**Autofix:** есть.

Последняя непустая строка каждой секции докстринга не должна заканчиваться точкой. Точки внутри секции между предложениями разрешены. Точка после цифры и точка внутри известных сокращений вроде `т.д.` / `т.п.` не считается завершающей точкой предложения.

## SK604 — DocstringRequiresCyrillic

**Уровень:** обычный.  
**Autofix:** нет.

Докстринг не должен быть полностью английским. Если в докстринге есть буквы, но нет кириллицы, SKLint считает его неподходящим.

## SK605 — DocstringProcessStyle

**Уровень:** обычный.  
**Autofix:** нет.

Описание функции, метода или класса должно формулироваться как процесс/состояние, а не как действие-глагол: `Ожидание обновления`, а не `Ожидать обновления`.

## SK606 — DocstringUnknownSection

**Уровень:** обычный.  
**Autofix:** нет.

Разрешены только секции `Args`, `Attributes`, `Returns`, `Yields`, `Raises`. Секция `Yields` используется для генераторов. Другие секции запрещены.

## SK607 — NestedDocstringBlankLine

**Уровень:** обычный.  
**Autofix:** есть.

Докстринги вложенных функций, методов и классов не должны содержать пустые строки.

## SK608 — NestedDocstringCanBeOneLine

**Уровень:** обычный.  
**Autofix:** есть.

Если докстринг вложенного объекта уже фактически однострочный и помещается в 72 символа вместе с кавычками, он должен быть записан в одну строку.

## SK609 — FinalConstantMissingDocstring

**Уровень:** обычный.  
**Autofix:** есть только как явное действие, создающее заготовку; массовый форматтер его не применяет.

Константа с аннотацией `Final` должна иметь строку-докстринг с описанием сразу после объявления. Если значение константы записано многострочно, докстринг должен идти сразу после завершения всего присваивания.

## SK610 — FinalConstantDocstringCanBeOneLine

**Уровень:** обычный.  
**Autofix:** есть.

Короткий докстринг константы должен быть записан в одну строку.

## SK611 — ModuleDocstringCanBeOneLine

**Уровень:** обычный.  
**Autofix:** есть.

Короткий докстринг модуля в начале файла должен быть записан в одну строку.

## SK612 — PublicDocstringQuotesOwnLines

**Уровень:** обычный.  
**Autofix:** частичный.

У невложенных функций, методов и классов открывающие и закрывающие тройные кавычки должны находиться на отдельных строках.

## SK613 — BlankLineAfterDocstring

**Уровень:** обычный.  
**Autofix:** есть.

После многострочного докстринга функции, метода или класса должна идти пустая строка. Однострочные докстринги вида `"""Описание"""` и функции/методы-заглушки с `...` это правило не проверяет.

## SK614 — DocstringMissingDescription

**Уровень:** обычный.  
**Autofix:** есть только как явное действие, создающее заготовку; массовый форматтер его не применяет.

У функции, метода или класса всегда должно быть описание помимо `Args`, `Attributes`, `Returns`, `Yields`, `Raises`.

## SK615 — DocstringDescriptionSectionGap

**Уровень:** обычный.  
**Autofix:** есть.

Если после описания идут секции, между описанием и первой секцией нужна пустая строка внутри докстринга.

## SK616 — DocstringRedundantObjectPrefix

**Уровень:** обычный.  
**Autofix:** есть.

Описание не должно начинаться со слов `Метод`, `Функция`, `Класс`.

## SK617 — DocstringCyrillicSentenceCapitalized

**Уровень:** обычный.  
**Autofix:** есть для первой буквы.

Кириллические предложения должны начинаться с большой буквы. Точка после цифры и точка внутри известных сокращений вроде `т.д.` / `т.п.` не начинает новое предложение.

## SK618 — DocstringTrailingWhitespace

**Уровень:** обычный.  
**Autofix:** есть.

В конце строк докстринга запрещены пробелы и табы, кроме ровно двух пробелов перед переносом строки для намеренного markdown-переноса.

## SK619 — DataclassAttributesMissingInherited

**Уровень:** обычный.  
**Autofix:** безопасно перестраивает существующую полную секцию `Attributes`; добавление отсутствующих смысловых описаний доступно только как явное действие и не применяется массовым форматтером.

Для `dataclass`/`dataclass_transform`-структур секция `Attributes` должна перечислять все поля, включая унаследованные поля базовых классов из текущего файла, и идти в том же порядке, в котором эти поля формируются после наследования.

## SK620 — DataclassAttributeTypeMismatch

**Уровень:** обычный.  
**Autofix:** есть: переписывает типы в `Attributes` по фактическим аннотациям dataclass-полей.

Типы атрибутов в секции `Attributes` должны совпадать с аннотациями полей dataclass, включая унаследованные поля. При сравнении игнорируются inline-комментарии в аннотациях полей и различия между одинарными и двойными кавычками в строковых `Literal`.

## SK621 — ModuleDocstringNoGapBeforeCode

**Уровень:** обычный.  
**Autofix:** есть.

После докстринга модуля не должно быть пустых строк перед следующим кодом.

## SK622 — FunctionSectionsNoBlankLinesBetween

**Уровень:** обычный.  
**Autofix:** есть.

В докстрингах функций и методов между `Args`, `Returns`, `Yields`, `Raises` не должно быть пустых строк.

## SK623 — FunctionSectionOrder

**Уровень:** обычный.  
**Autofix:** есть.

Секции функций и методов должны идти строго в порядке `Args`, `Returns`, `Yields`, `Raises`. Автофикс переставляет уже существующие секции в правильный порядок, не генерируя смысловое описание.

## SK624 — DocstringItemContinuationStartsNextLine

**Уровень:** обычный.  
**Autofix:** есть.

Если описание аргумента, атрибута или исключения не помещается на одну строку, оно должно начинаться со следующей строки после дополнительного отступа.

## SK701 — DynamicSelfAttributeOutsideInit

**Уровень:** обычный.  
**Autofix:** нет.

Предупреждает о динамическом атрибуте `self`, который вводится присваиванием через `self.<name> = ...` вне `__init__` / `__post_init__` / `__setstate__` и не объявлен в теле класса или аннотациях класса. Обычные чтения такого атрибута не дублируются отдельными предупреждениями.

Граница правила намеренно узкая: оно не дублирует случай `obj.unknown = ...`, который Pyright/Pylance уже умеет диагностировать как `reportAttributeAccessIssue`. `SK701` закрывает другой случай: атрибут создаётся внутри обычного метода через `self.generated = ...`, после чего используется как будто он является нормальным полем экземпляра. Pyright в строгом режиме оставляет такой файл без diagnostics, поэтому именно этот паттерн считается кандидатом на “белый” динамический элемент.

В VSCode расширение дополнительно фильтрует `SK701`: если рядом уже есть diagnostics Pyright/Pylance или атрибут покрыт semantic token, предупреждение SKLint скрывается.

Пример:

```python
class Box:
    def attach(self) -> None:
        self.generated = 1

    def read(self) -> int:
        return self.generated
```

Как исправить:

```python
class Box:
    generated: int

    def attach(self) -> None:
        self.generated = 1

    def read(self) -> int:
        return self.generated
```


## SK702 — DynamicObjectAttributeAssignment

**Уровень:** обычный.  
**Autofix:** нет.

Атрибуты объектов, созданных из известных динамических контейнеров, должны быть объявлены в классе заранее. Правило покрывает случай, когда Pyright/Pylance молчит из-за динамического `__getattr__` / `__setattr__`, а имя атрибута в VSCode остаётся “белым”.

Граница правила: `SK702` проверяет только локально видимые классы, экземпляры которых созданы прямым вызовом конструктора (`obj = ClassName()`), и только классы с собственными динамическими attribute hooks или с базами `DataDict` / `BaseConcConfig`. Обычные классы без таких hooks не проверяются, потому что их неизвестные атрибуты Pyright уже диагностирует сам.

Пример нарушения:

```python
class Common(BaseConcConfig):
    pass

common = Common()
common.camera_config = CameraConfig()
```

Как исправить:

```python
class Common(BaseConcConfig):
    camera_config: CameraConfig | None = None
```

В VSCode расширение дополнительно фильтрует `SK702`: если рядом уже есть diagnostics Pyright/Pylance или атрибут покрыт semantic token, предупреждение SKLint скрывается.

## SK900 — UnusedSuppression

**Уровень:** обычный.  
**Autofix:** безопасный selector-level fix.

Сообщает о подавлениях SKLint, которые больше не подавляют актуальные предупреждения. Использование считается отдельно для каждого selector-а; при перекрытии broad/exact suppressions ответственность получает наиболее локальное и специфичное подавление. Formatter удаляет только лишний SKLint selector/segment и сохраняет соседние Ruff/Flake8-коды и обычные комментарии. Поддерживаются `noqa`, `sklint: ignore/disable`, catch-all suppressions, `enable`, prefix selectors и алиасы `DOCxxx ↔ SKDxxx`. Bare `# noqa` без явного SKLint selector-а не считается подавлением SKLint.

## SK401 — AssignmentOperatorSpacing

**Уровень:** обычный.  

Оператор присваивания `=` должен иметь пробелы с обеих сторон. Исключения: `==`, `!=`, `<=`, `>=`, `:=` и случай, когда `=` стоит последним значимым символом строки перед переносом.

Autofix: безопасно нормализует пробелы вокруг `=`.

Пробелы вокруг арифметических, битовых, сравнительных и boolean-операторов не дублируются: их следует закрывать Ruff/pycodestyle правилами `E225`, `E226`, `E227` и `E228`.

Современный typing-синтаксис тоже не дублируется: используйте Ruff `UP006`, `UP007`, `UP045` и `UP040`.

## SK403 — MultilineBracketItemLayout

**Уровень:** обычный.  

Если скобочная конструкция раскрыта на несколько строк, элементы внутри должны идти с отступом и по одному на строку. Однострочные вызовы вида `Point(x, y)` не затрагиваются; правило срабатывает только для раскрытых конструкций вида `Point(
    x, y
)`. Подчёркивается первый лишний элемент на строке, например `y`.

Autofix: доступна только unsafe/manual suggestion для простых случаев; bulk formatter её не применяет автоматически.

## SK404 — TrailingComma

**Уровень:** обычный.  

Висящая запятая — это запятая, после которой нет следующего элемента и сразу закрывается `)`, `]` или `}`. Обычные многострочные сигнатуры, import-блоки и одноэлементные tuple-литералы вида `(value,)` не нарушают это правило.

Autofix: удаляет висящую запятую.

## SK502 — FromImportOnly

**Уровень:** обычный.  

Импорты должны быть выполнены через форму `from module import name`. Исключение: `import sys`, если в файле используется `sys.platform` или `sys.version_info` для runtime-ветвления.

Autofix: отсутствует — безопасно вывести требуемый `from ... import ...` для всех вариантов импорта и областей видимости нельзя.

## SK503 — PreferSysPlatform

**Уровень:** обычный.  

`os.name` менее информативен, чем `sys.platform`. Для проверок платформы используйте `sys.platform`.

Autofix: отсутствует — замена требует согласованного изменения/import-а `sys` и проверки shadowing.

## SK504 — DirectSysPlatformImport

**Уровень:** обычный.  

`from sys import platform` запрещён для проверок платформы. Pylance/Pyright лучше понимает ветвления в форме `import sys` + `if sys.platform ...`.

Autofix: переписывает импорт и условия `if platform ...` / `elif platform ...`.

## SK505 — DefinitionOrder

**Уровень:** обычный.  

Функции, классы и методы должны быть объявлены выше мест, где они используются. Порядок специальных методов `__new__` / `__init__` / `__post_init__` проверяется отдельным правилом `SK509`.

Autofix: поддерживается встроенным форматировщиком. Форматировщик переставляет цельные блоки `def`/`class` с декораторами выше первого использования, если границы блока можно определить безопасно.

## SK509 — SpecialMethodOrder

**Уровень:** обычный.  

`__new__`, `__init__` и `__post_init__` должны идти перед обычными методами именно в таком порядке. `__new__` может отсутствовать, но если он есть, он должен идти раньше `__init__` и `__post_init__`.

Autofix: поддерживается встроенным форматировщиком. Форматировщик переставляет цельные блоки методов внутри класса в порядке `__new__`, `__init__`, `__post_init__`, затем остальные методы.

## SK506 — TryExceptFinallyForbidden

**Уровень:** обычный.  

`try`, `except` и `finally` запрещены в hot runtime-коде проекта, так как такая структура часто уводит управление в исключительный путь и усложняет оптимизацию.

Autofix: отсутствует.

## SK510 — ContextlibSuppressForbidden

**Уровень:** strict-only.  
**Autofix:** нет.

В строгом режиме запрещает `contextlib.suppress(...)`, включая `from contextlib import suppress`, алиасы импорта и форму `import contextlib as ...`. `suppress` скрывает исключительный путь управления по той же причине, по которой `SK506` предупреждает о `try`/`except`: исключение не должно бесшумно превращаться в обычное продолжение runtime-кода.

## SK507 — RaiseHotPath

**Уровень:** обычный.  

`raise` разрешён только в методах `__init__`, `__post_init__`, `run`, `close`, а также в приватных helper-методах, которые используются только этими методами текущего класса.

Autofix: отсутствует.


## SK508 — FutureAnnotationsImport

**Уровень:** обычный.  

`from __future__ import annotations` запрещён. Проект ориентируется на современный runtime и не должен менять поведение аннотаций через future-import.

Autofix: удаляет строку `from __future__ import annotations`, если она содержит только этот импорт.

## SK801 — InlineSingleUseVariable

**Уровень:** strict-only.  

Strict-only. Промежуточная переменная, которая используется ровно один раз за оставшийся lexical lifetime в текущей функции (или модуле) и может быть безопасно подставлена в непосредственно следующий логический statement, должна быть свернута. При подсчёте учитываются многострочные выражения и дополнительные последующие упоминания; при неоднозначности правило консервативно не предлагает автоматическое исправление.

Autofix: удаляет простое присваивание и подставляет выражение в полный следующий statement только после проверки, что имя больше нигде в оставшейся lexical scope не используется.

## SK802 — ReturnTernary

**Уровень:** strict-only.  

Strict-only. Последовательность `if condition: return a; return b` должна быть свернута в `return a if condition else b`.

Autofix: сворачивает простые return-ветки в тернарное выражение.

## SK803 — LoopComprehension

**Уровень:** strict-only.  

Strict-only. Простой цикл `append` в заранее созданный список должен быть заменён на list comprehension, если это делает код короче и эффективнее.

Autofix: преобразует `items = []; for x in xs: items.append(expr)` в `items = [expr for x in xs]`.

## SK804 — PublicAllTuple

**Уровень:** strict-only.  

Strict-only. Непустой модуль с публичными символами должен объявлять `__all__` именно как tuple: `__all__ = (...)`, не list.

Autofix: создаёт или переписывает простой `__all__` tuple template.

## SK805 — FileWideSuppression

**Уровень:** strict-only.  

Strict-only. В начале файла запрещены глобальные подавления предупреждений для всего файла. Правило проверяет пролог файла до первого кода, а также комментарии сразу после модульного докстринга. Локальные подавления на строках кода не запрещаются.

Запрещённые примеры:

```python
# ruff: noqa
# flake8: noqa
# pylint: disable=missing-module-docstring
# pyright: reportPrivateUsage=false
# type: ignore
# mypy: ignore-errors
# sklint: ignore=SK804
```

Autofix: безопасно удаляет только запрещённый file-wide suppression segment, сохраняя соседний обычный комментарий. Раскрывшиеся после этого реальные diagnostics проверяются на следующем formatter-round.


# Rust-native pydoclint port и дополнительные strict rules

`DOCxxx` selectors являются алиасами соответствующих `SKDxxx` для upstream-совместимости. Исключение — собственное расширение `SKD608`, у которого нет upstream `DOC608`.

## SKD001 — DocstringParseError

**Уровень:** strict-only.  \n**Autofix:** нет.

Докстринг не удалось корректно разобрать в выбранном/обнаруженном стиле; downstream pydoclint-диагностики для этого докстринга подавляются, чтобы не создавать каскад ложных ошибок.

## SKD002 — PythonSyntaxError

**Уровень:** normal.  \n**Autofix:** нет.

Python source подтверждён как синтаксически невалидный. Диагностика совместима с DOC002 и выдаётся на первой строке файла; infrastructure failure внешнего syntax oracle не считается syntax error и вместо этого получает `SK903`. Для `invalid non-printable character` выполняется upstream-compatible retry после удаления известных invisible Unicode characters.

## SKD003 — DocstringStyleMismatch

**Уровень:** strict-only.  \n**Autofix:** нет.

При включённом `check-style-mismatch` обнаруженный стиль докстринга отличается от configured pydoclint style.

## SKD101 — DocstringArgumentsMissing

**Уровень:** strict-only.  \n**Autofix:** условный safe structural fix.

В секции параметров отсутствуют аргументы из сигнатуры. Formatter может создать полностью отсутствующую секцию из AST, но не реконструирует частично авторскую секцию.

## SKD102 — DocstringArgumentsExtra

**Уровень:** strict-only.  \n**Autofix:** нет.

В докстринге задокументированы параметры, которых нет в эффективной сигнатуре.

## SKD103 — DocstringArgumentsMismatch

**Уровень:** strict-only.  \n**Autofix:** нет.

Набор имён параметров докстринга и эффективной сигнатуры различается.

## SKD104 — DocstringArgumentOrder

**Уровень:** strict-only.  \n**Autofix:** условный lossless fix для Google/NumPy.

Параметры описаны не в порядке эффективной сигнатуры. Formatter переставляет исходные item-блоки целиком только при однозначном 1:1 соответствии; Sphinx автоматически не переставляется.

## SKD105 — DocstringArgumentTypeMismatch

**Уровень:** strict-only.  \n**Autofix:** условный safe type-rewrite для Google/NumPy/Sphinx.

Тип параметра в докстринге не соответствует аннотации сигнатуры с учётом нормализации pydoclint.

## SKD106 — SignatureArgumentTypesMissingAll

**Уровень:** strict-only.  \n**Autofix:** нет.

Конфигурация требует type hints в сигнатуре, но они отсутствуют у всех проверяемых аргументов.

## SKD107 — SignatureArgumentTypesMissingSome

**Уровень:** strict-only.  \n**Autofix:** нет.

Конфигурация требует type hints в сигнатуре, но они отсутствуют у части аргументов.

## SKD108 — SignatureArgumentTypesForbidden

**Уровень:** strict-only.  \n**Autofix:** нет.

Конфигурация запрещает type hints в сигнатуре, но они присутствуют.

## SKD109 — DocstringArgumentTypesMissingAll

**Уровень:** strict-only.  \n**Autofix:** условный safe type-rewrite для Google/NumPy/Sphinx.

Конфигурация требует типы параметров в докстринге, но они отсутствуют у всех параметров.

## SKD110 — DocstringArgumentTypesMissingSome

**Уровень:** strict-only.  \n**Autofix:** условный safe type-rewrite для Google/NumPy/Sphinx.

Конфигурация требует типы параметров в докстринге, но они отсутствуют у части параметров.

## SKD111 — DocstringArgumentTypesForbidden

**Уровень:** strict-only.  \n**Autofix:** условный safe type-rewrite для Google/NumPy/Sphinx.

Конфигурация запрещает type hints параметров в докстринге, но они присутствуют.

## SKD201 — ReturnSectionMissing

**Уровень:** strict-only.  \n**Autofix:** условный safe structural fix.

Функция возвращает значение, но секция `Returns` отсутствует. Formatter создаёт её только когда return type можно безопасно вывести из AST.

## SKD202 — ReturnSectionUnnecessary

**Уровень:** strict-only.  \n**Autofix:** нет.

В докстринге есть `Returns`, хотя функция не имеет документируемого return.

## SKD203 — ReturnTypeMismatch

**Уровень:** strict-only.  \n**Autofix:** условный safe type-rewrite для Google/NumPy/Sphinx.

Тип в `Returns` не соответствует return annotation.

## SKD301 — InitDocstringForbidden

**Уровень:** opt-in.  \n**Autofix:** нет.

Отдельный docstring у `__init__` запрещён. В SKLint это правило намеренно выключено по умолчанию даже в strict-mode и активируется только явным `select` (`DOC301`/`SKD301` или явный prefix selector).

## SKD302 — ClassReturnSectionForbidden

**Уровень:** strict-only.  \n**Autofix:** нет.

Class docstring не должен содержать `Returns` для конструктора.

## SKD303 — InitReturnSectionForbidden

**Уровень:** strict-only.  \n**Autofix:** нет.

Docstring `__init__` не должен содержать `Returns`.

## SKD304 — ClassArgumentSectionForbidden

**Уровень:** strict-only.  \n**Autofix:** нет.

Когда документацией аргументов владеет `__init__`, секция аргументов class docstring запрещена.

## SKD305 — ClassRaisesSectionForbidden

**Уровень:** strict-only.  \n**Autofix:** нет.

Когда документацией исключений владеет `__init__`, `Raises` в class docstring запрещён.

## SKD306 — ClassYieldSectionForbidden

**Уровень:** strict-only.  \n**Autofix:** нет.

Class docstring не должен содержать `Yields` для конструктора.

## SKD307 — InitYieldSectionForbidden

**Уровень:** strict-only.  \n**Autofix:** нет.

Docstring `__init__` не должен содержать `Yields`.

## SKD402 — YieldSectionMissing

**Уровень:** strict-only.  \n**Autofix:** условный safe structural fix.

Generator содержит `yield`, но секция `Yields` отсутствует. Formatter создаёт её только когда yield type можно вывести из annotation/AST.

## SKD403 — YieldSectionUnnecessary

**Уровень:** strict-only.  \n**Autofix:** нет.

В докстринге есть `Yields`, хотя функция не является документируемым generator.

## SKD404 — YieldTypeMismatch

**Уровень:** strict-only.  \n**Autofix:** условный safe type-rewrite для Google/NumPy/Sphinx.

Тип в `Yields` не соответствует generator annotation.

## SKD405 — GeneratorReturnAnnotationMismatch

**Уровень:** strict-only.  \n**Autofix:** нет.

Функция одновременно возвращает значение и yield-ит, но annotation не соответствует Generator-подобной семантике.

## SKD501 — RaisesSectionMissing

**Уровень:** strict-only.  \n**Autofix:** условный safe structural fix.

Функция явно поднимает исключения, но `Raises` отсутствует. Formatter может создать полностью отсутствующую секцию из известных AST exception names.

## SKD502 — RaisesSectionUnnecessary

**Уровень:** strict-only.  \n**Autofix:** нет.

Докстринг объявляет исключения, для которых SKLint не видит runtime exception path. Анализ учитывает явные `raise`/`assert` и распространённые implicit paths вроде division/modulo/indexing.

Полный interprocedural exception-flow через произвольные callees пока не строится. Для wrapper-heavy проектов можно явно включить `[tool.sklint] allow_documented_propagated_exceptions = true`: тогда наличие call expression считается допустимым основанием для документированного propagated `Raises:` без ложного SKD502. По умолчанию опция выключена, чтобы сохранить строгую pydoclint-compatible семантику. Pure signature stubs с `...` не проверяются по direct-body `Raises` logic: документация исключений описывает отсутствующую реализацию, а не ellipsis-body.

## SKD503 — RaisedExceptionsMismatch

**Уровень:** strict-only.  \n**Autofix:** нет.

Набор документированных исключений не совпадает с реально поднимаемыми; дубликаты также считаются ошибкой. Для variable re-raise SKLint выводит тип из annotations параметров/локальных переменных, `self.attr` и атрибутов локально созданного typed object (`capture = Capture(); raise capture.error`), убирая `None` из Optional/union annotation.

## SKD504 — AssertRaisesSectionMissing

**Уровень:** strict-only.  \n**Autofix:** условный safe structural fix.

При включённой соответствующей опции `assert` требует `AssertionError` в `Raises`; полностью отсутствующую секцию formatter может создать.

## SKD601 — ClassAttributesMissing

**Уровень:** strict-only.  \n**Autofix:** условный safe structural fix.

В class docstring отсутствуют эффективные class/dataclass/PEP 681 attributes. Formatter может создать полностью отсутствующую секцию из модели полей.

## SKD602 — ClassAttributesExtra

**Уровень:** strict-only.  \n**Autofix:** нет.

В class docstring задокументированы атрибуты, которых нет в effective field model.

## SKD603 — ClassAttributesMismatch

**Уровень:** strict-only.  \n**Autofix:** нет.

Набор документированных class attributes отличается от effective field model.

## SKD604 — ClassAttributeOrder

**Уровень:** strict-only.  \n**Autofix:** условный lossless fix для Google/NumPy.

Порядок class attributes отличается от effective field order, включая наследуемые dataclass/PEP 681 fields. Formatter переставляет оригинальные блоки только при однозначном соответствии.

## SKD605 — ClassAttributeTypeMismatch

**Уровень:** strict-only.  \n**Autofix:** условный safe type-rewrite для Google/NumPy/Sphinx.

Тип class attribute в докстринге отличается от аннотации/эффективного default-aware field type. Для `ctypes.Structure`/`Union` явная Python annotation имеет приоритет над raw `_fields_` storage declaration. `POINTER(T)` нормализуется для сравнения как `POINTER[T]`; неоднозначный storage-only ctypes mismatch не получает safe docstring rewrite.

## SKD606 — InlineClassAttributeDocForbidden

**Уровень:** strict-only.  \n**Autofix:** нет.

Inline-документация class attribute запрещена, когда configured policy требует class docstring section.

## SKD607 — ClassAttributesSectionForbidden

**Уровень:** strict-only.  \n**Autofix:** нет.

`Attributes` section запрещена, когда configured policy требует inline class-variable docs.

## SKD608 — DocstringArgumentDescriptionEmpty

**Уровень:** strict-only.  \n**Autofix:** нет.

Каждый документированный параметр/аргумент обязан иметь непустое описание. Поддерживаются Google, NumPy и Sphinx, включая multiline descriptions.

## SK901 — MagicNumericConstant

Числа внутри выражения, присваиваемого явно именованной константе (`UPPER_CASE` или аннотация `Final`), считаются самодокументируемым контекстом и не требуют дополнительного `SK901`-подавления.

**Уровень:** strict-only.  \n**Autofix:** нет.

Запрещает неочевидные числовые константы по объединённой семантике Ruff PLR2004 и WPS432. Разрешены self-documenting literal contexts/common values и специальные `sys.version*` comparisons; всё тело точного `if __name__ == "__main__":` исключено.



## SK902 — PartialAstAnalysis

**Уровень:** normal.  \n**Autofix:** нет.

Python-файл синтаксически валиден для доступного изолированного CPython, но embedded parser SKLint не смог построить полный AST даже после compatibility rewrite. Такой файл не считается полностью проанализированным: diagnostic предотвращает silent false-green для AST-dependent rules. Compatibility layer покрывает используемые StableKite формы PEP 701/695/696, включая nested/multiline f-strings, generic classes/functions и default type parameters; SK902 остаётся safety net для будущих grammar gaps.

## SK903 — SyntaxOracleUnavailable

**Уровень:** normal.  \n**Autofix:** нет.

Embedded parser не смог разобрать файл, а внешний изолированный syntax oracle (`SKLINT_PYTHON`, `python3`, `python` или `py -3`) отсутствует, завершился инфраструктурной ошибкой либо превысил timeout. Это состояние не трактуется как syntax error исходника: `SKD002` выдаётся только после подтверждённого syntax rejection.
