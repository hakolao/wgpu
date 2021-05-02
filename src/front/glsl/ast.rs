use super::{super::Typifier, constants::ConstantSolver, error::ErrorKind, TokenMetadata};
use crate::{
    proc::ResolveContext, Arena, ArraySize, BinaryOperator, Binding, Constant, Expression,
    FastHashMap, Function, FunctionArgument, GlobalVariable, Handle, Interpolation, Module,
    RelationalFunction, ResourceBinding, Sampling, ShaderStage, Statement, StorageClass, Type,
    UnaryOperator,
};

#[derive(Debug)]
pub struct Program<'a> {
    pub version: u16,
    pub profile: Profile,
    pub entry_points: &'a FastHashMap<String, ShaderStage>,
    pub lookup_function: FastHashMap<String, Handle<Function>>,
    pub lookup_type: FastHashMap<String, Handle<Type>>,
    pub lookup_global_variables: FastHashMap<String, Handle<GlobalVariable>>,
    pub lookup_constants: FastHashMap<String, Handle<Constant>>,
    pub module: Module,
}

impl<'a> Program<'a> {
    pub fn new(entry_points: &'a FastHashMap<String, ShaderStage>) -> Program<'a> {
        Program {
            version: 0,
            profile: Profile::Core,
            entry_points,
            lookup_function: FastHashMap::default(),
            lookup_type: FastHashMap::default(),
            lookup_global_variables: FastHashMap::default(),
            lookup_constants: FastHashMap::default(),
            module: Module::default(),
        }
    }

    pub fn binary_expr(
        &mut self,
        function: &mut Function,
        op: BinaryOperator,
        left: &ExpressionRule,
        right: &ExpressionRule,
    ) -> ExpressionRule {
        ExpressionRule::from_expression(function.expressions.append(Expression::Binary {
            op,
            left: left.expression,
            right: right.expression,
        }))
    }

    pub fn unary_expr(
        &mut self,
        function: &mut Function,
        op: UnaryOperator,
        tgt: &ExpressionRule,
    ) -> ExpressionRule {
        ExpressionRule::from_expression(function.expressions.append(Expression::Unary {
            op,
            expr: tgt.expression,
        }))
    }

    /// Helper function to insert equality expressions, this handles the special
    /// case of `vec1 == vec2` and `vec1 != vec2` since in the IR they are
    /// represented as `all(equal(vec1, vec2))` and `any(notEqual(vec1, vec2))`
    pub fn equality_expr(
        &mut self,
        context: &mut FunctionContext,
        equals: bool,
        left: &ExpressionRule,
        right: &ExpressionRule,
    ) -> Result<ExpressionRule, ErrorKind> {
        let left_is_vector = match *self.resolve_type(context, left.expression)? {
            crate::TypeInner::Vector { .. } => true,
            _ => false,
        };

        let right_is_vector = match *self.resolve_type(context, right.expression)? {
            crate::TypeInner::Vector { .. } => true,
            _ => false,
        };

        let (op, fun) = match equals {
            true => (BinaryOperator::Equal, RelationalFunction::All),
            false => (BinaryOperator::NotEqual, RelationalFunction::Any),
        };

        let expr = ExpressionRule::from_expression(context.function.expressions.append(
            Expression::Binary {
                op,
                left: left.expression,
                right: right.expression,
            },
        ));

        Ok(if left_is_vector && right_is_vector {
            ExpressionRule::from_expression(context.function.expressions.append(
                Expression::Relational {
                    fun,
                    argument: expr.expression,
                },
            ))
        } else {
            expr
        })
    }

    pub fn resolve_type<'b>(
        &'b mut self,
        context: &'b mut FunctionContext,
        handle: Handle<Expression>,
    ) -> Result<&'b crate::TypeInner, ErrorKind> {
        let resolve_ctx = ResolveContext {
            constants: &self.module.constants,
            global_vars: &self.module.global_variables,
            local_vars: &context.function.local_variables,
            functions: &self.module.functions,
            arguments: &context.function.arguments,
        };
        match context.typifier.grow(
            handle,
            &context.function.expressions,
            &mut self.module.types,
            &resolve_ctx,
        ) {
            //TODO: better error report
            Err(error) => Err(ErrorKind::SemanticError(
                format!("Can't resolve type: {:?}", error).into(),
            )),
            Ok(()) => Ok(context.typifier.get(handle, &self.module.types)),
        }
    }

    pub fn solve_constant(
        &mut self,
        expressions: &Arena<Expression>,
        root: Handle<Expression>,
    ) -> Result<Handle<Constant>, ErrorKind> {
        let mut solver = ConstantSolver {
            types: &self.module.types,
            expressions,
            constants: &mut self.module.constants,
        };

        solver
            .solve(root)
            .map_err(|_| ErrorKind::SemanticError("Can't solve constant".into()))
    }

    pub fn type_size(&self, ty: Handle<Type>) -> Result<u8, ErrorKind> {
        Ok(match self.module.types[ty].inner {
            crate::TypeInner::Scalar { width, .. } => width,
            crate::TypeInner::Vector { size, width, .. } => size as u8 * width,
            crate::TypeInner::Matrix {
                columns,
                rows,
                width,
            } => columns as u8 * rows as u8 * width,
            crate::TypeInner::Pointer { .. } => {
                return Err(ErrorKind::NotImplemented("type size of pointer"))
            }
            crate::TypeInner::ValuePointer { .. } => {
                return Err(ErrorKind::NotImplemented("type size of value pointer"))
            }
            crate::TypeInner::Array { size, stride, .. } => {
                stride as u8
                    * match size {
                        ArraySize::Dynamic => {
                            return Err(ErrorKind::NotImplemented("type size of dynamic array"))
                        }
                        ArraySize::Constant(constant) => {
                            match self.module.constants[constant].inner {
                                crate::ConstantInner::Scalar { width, .. } => width,
                                crate::ConstantInner::Composite { .. } => {
                                    return Err(ErrorKind::NotImplemented(
                                        "type size of array with composite item size",
                                    ))
                                }
                            }
                        }
                    }
            }
            crate::TypeInner::Struct { .. } => {
                return Err(ErrorKind::NotImplemented("type size of struct"))
            }
            crate::TypeInner::Image { .. } => {
                return Err(ErrorKind::NotImplemented("type size of image"))
            }
            crate::TypeInner::Sampler { .. } => {
                return Err(ErrorKind::NotImplemented("type size of sampler"))
            }
        })
    }
}

#[derive(Debug)]
pub enum Profile {
    Core,
}

#[derive(Debug)]
pub struct FunctionContext<'function> {
    pub function: &'function mut Function,
    //TODO: Find less allocation heavy representation
    pub scopes: Vec<FastHashMap<String, VariableReference>>,
    pub lookup_global_var_exps: FastHashMap<String, VariableReference>,
    pub lookup_constant_exps: FastHashMap<String, VariableReference>,
    pub typifier: Typifier,
}

impl<'function> FunctionContext<'function> {
    pub fn new(function: &'function mut Function) -> Self {
        FunctionContext {
            function,
            scopes: vec![FastHashMap::default()],
            lookup_global_var_exps: FastHashMap::default(),
            lookup_constant_exps: FastHashMap::default(),
            typifier: Typifier::new(),
        }
    }

    pub fn lookup_local_var(&self, name: &str) -> Option<VariableReference> {
        for scope in self.scopes.iter().rev() {
            if let Some(var) = scope.get(name) {
                return Some(var.clone());
            }
        }
        None
    }

    #[cfg(feature = "glsl-validate")]
    pub fn lookup_local_var_current_scope(&self, name: &str) -> Option<VariableReference> {
        if let Some(current) = self.scopes.last() {
            current.get(name).cloned()
        } else {
            None
        }
    }

    pub fn clear_scopes(&mut self) {
        self.scopes.clear();
        self.scopes.push(FastHashMap::default());
    }

    /// Add variable to current scope
    pub fn add_local_var(&mut self, name: String, expr: Handle<Expression>) {
        if let Some(current) = self.scopes.last_mut() {
            let load = self
                .function
                .expressions
                .append(Expression::Load { pointer: expr });

            (*current).insert(
                name,
                VariableReference {
                    expr,
                    load: Some(load),
                },
            );
        }
    }

    /// Add function argument to current scope
    pub fn add_function_arg(&mut self, name: Option<String>, ty: Handle<Type>) {
        let index = self.function.arguments.len();
        self.function.arguments.push(FunctionArgument {
            name: name.clone(),
            ty,
            binding: None,
        });

        if let Some(name) = name {
            if let Some(current) = self.scopes.last_mut() {
                let expr = self
                    .function
                    .expressions
                    .append(Expression::FunctionArgument(index as u32));

                (*current).insert(name, VariableReference { expr, load: None });
            }
        }
    }

    /// Add new empty scope
    pub fn push_scope(&mut self) {
        self.scopes.push(FastHashMap::default());
    }

    pub fn remove_current_scope(&mut self) {
        self.scopes.pop();
    }

    pub fn resolve(
        &mut self,
        program: &mut Program,
        expr: Expr,
        lhs: bool,
    ) -> Result<Handle<Expression>, ErrorKind> {
        Ok(match expr.kind {
            ExprKind::Access { base, index } => {
                let base = self.resolve(program, *base, lhs)?;
                let index = self.resolve(program, *index, false)?;

                self.function
                    .expressions
                    .append(Expression::Access { base, index })
            }
            ExprKind::Select { base, field } => {
                let base = self.resolve(program, *base, lhs)?;

                program.field_selection(self, base, &field, expr.meta)?
            }
            ExprKind::Constant(constant) => self
                .function
                .expressions
                .append(Expression::Constant(constant)),
            ExprKind::Binary { left, op, right } => {
                let left = self.resolve(program, *left, false)?;
                let right = self.resolve(program, *right, false)?;

                self.function
                    .expressions
                    .append(Expression::Binary { left, op, right })
            }
            ExprKind::Unary { op, expr } => {
                let expr = self.resolve(program, *expr, false)?;

                self.function
                    .expressions
                    .append(Expression::Unary { op, expr })
            }
            ExprKind::Variable(var) => {
                if lhs {
                    var.expr
                } else {
                    var.load.unwrap_or(var.expr)
                }
            }
            ExprKind::Call(_) => todo!(),
            ExprKind::Conditional {
                condition,
                accept,
                reject,
            } => {
                let condition = self.resolve(program, *condition, false)?;
                let accept = self.resolve(program, *accept, false)?;
                let reject = self.resolve(program, *reject, false)?;

                self.function.expressions.append(Expression::Select {
                    condition,
                    accept,
                    reject,
                })
            }
            ExprKind::Assign { tgt, value } => {
                let pointer = self.resolve(program, *tgt, false)?;
                let value = self.resolve(program, *value, false)?;

                self.function.body.push(Statement::Store { pointer, value });

                value
            }
        })
    }
}

#[derive(Debug)]
pub struct ExpressionRule {
    pub expression: Handle<Expression>,
    pub statements: Vec<Statement>,
    pub sampler: Option<Handle<Expression>>,
}

impl ExpressionRule {
    pub fn from_expression(expression: Handle<Expression>) -> ExpressionRule {
        ExpressionRule {
            expression,
            statements: vec![],
            sampler: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct VariableReference {
    pub expr: Handle<Expression>,
    pub load: Option<Handle<Expression>>,
}

#[derive(Debug)]
pub struct Expr {
    pub kind: ExprKind,
    pub meta: TokenMetadata,
}

#[derive(Debug)]
pub enum ExprKind {
    Access {
        base: Box<Expr>,
        index: Box<Expr>,
    },
    Select {
        base: Box<Expr>,
        field: String,
    },
    Constant(Handle<Constant>),
    Binary {
        left: Box<Expr>,
        op: BinaryOperator,
        right: Box<Expr>,
    },
    Unary {
        op: UnaryOperator,
        expr: Box<Expr>,
    },
    Variable(VariableReference),
    Call(FunctionCall),
    Conditional {
        condition: Box<Expr>,
        accept: Box<Expr>,
        reject: Box<Expr>,
    },
    Assign {
        tgt: Box<Expr>,
        value: Box<Expr>,
    },
}

#[derive(Debug)]
pub enum TypeQualifier {
    StorageQualifier(StorageQualifier),
    Interpolation(Interpolation),
    ResourceBinding(ResourceBinding),
    Binding(Binding),
    Sampling(Sampling),
    Layout(StructLayout),
    EarlyFragmentTests,
}

#[derive(Debug)]
pub struct VarDeclaration {
    pub type_qualifiers: Vec<TypeQualifier>,
    pub ids_initializers: Vec<(Option<String>, Option<ExpressionRule>)>,
    pub ty: Handle<Type>,
}

#[derive(Debug)]
pub enum FunctionCallKind {
    TypeConstructor(Handle<Type>),
    Function(String),
}

#[derive(Debug)]
pub struct FunctionCall {
    pub kind: FunctionCallKind,
    pub args: Vec<Expr>,
}

#[derive(Debug, Clone, Copy)]
pub enum StorageQualifier {
    StorageClass(StorageClass),
    Input,
    Output,
    InOut,
    Const,
}

#[derive(Debug, Clone)]
pub enum StructLayout {
    Std140,
}
