# SKLint 0.1.44 — SK801 precedence safety

SK801 previously substituted a single-use temporary variable as raw text. When the
right-hand side was a conditional expression and the temporary was called, this
changed Python precedence:

```python
socket_cls = A if condition else B
return socket_cls(1)
```

was incorrectly fixed as:

```python
return A if condition else B(1)
```

SKLint now classifies RHS expressions conservatively. Identifiers, literals,
bracketed displays and primary chains can be inserted directly. Other expressions
are grouped whenever they are inserted into a larger expression:

```python
return (A if condition else B)(1)
```

The same protection covers binary and unary expressions and implicit adjacent
string literal concatenation. A direct `return temporary` or `yield temporary`
keeps the original RHS without redundant parentheses because no surrounding
operator can change its meaning.
