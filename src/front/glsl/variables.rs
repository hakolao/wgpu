use crate::{Expression, Handle, Type, TypeInner, VectorSize};

use super::ast::*;
use super::error::ErrorKind;
use super::token::TokenMetadata;

impl Program<'_> {
    pub fn lookup_variable(
        &mut self,
        context: &mut FunctionContext,
        name: &str,
    ) -> Result<Option<VariableReference>, ErrorKind> {
        if let Some(local_var) = context.lookup_local_var(name) {
            return Ok(Some(local_var));
        }
        if let Some(global_var) = context.lookup_global_var_exps.get(name) {
            return Ok(Some(global_var.clone()));
        }
        if let Some(constant) = context.lookup_constant_exps.get(name) {
            return Ok(Some(constant.clone()));
        }
        match name {
            "gl_Position" => {
                todo!()
            }
            "gl_VertexIndex" => {
                todo!()
            }
            "gl_InstanceIndex" => {
                todo!()
            }
            _ => Ok(None),
        }
    }

    pub fn field_selection(
        &mut self,
        context: &mut FunctionContext,
        expression: Handle<Expression>,
        name: &str,
        meta: TokenMetadata,
    ) -> Result<Handle<Expression>, ErrorKind> {
        match *self.resolve_type(context, expression)? {
            TypeInner::Struct { ref members, .. } => {
                let index = members
                    .iter()
                    .position(|m| m.name == Some(name.into()))
                    .ok_or_else(|| ErrorKind::UnknownField(meta, name.into()))?;
                Ok(context
                    .function
                    .expressions
                    .append(Expression::AccessIndex {
                        base: expression,
                        index: index as u32,
                    }))
            }
            // swizzles (xyzw, rgba, stpq)
            TypeInner::Vector { size, kind, width } => {
                let check_swizzle_components = |comps: &str| {
                    name.chars()
                        .map(|c| {
                            comps
                                .find(c)
                                .and_then(|i| if i < size as usize { Some(i) } else { None })
                        })
                        .fold(Some(Vec::<usize>::new()), |acc, cur| {
                            cur.and_then(|i| {
                                acc.map(|mut v| {
                                    v.push(i);
                                    v
                                })
                            })
                        })
                };

                let indices = check_swizzle_components("xyzw")
                    .or_else(|| check_swizzle_components("rgba"))
                    .or_else(|| check_swizzle_components("stpq"));

                if let Some(v) = indices {
                    let components: Vec<Handle<Expression>> = v
                        .iter()
                        .map(|idx| {
                            context
                                .function
                                .expressions
                                .append(Expression::AccessIndex {
                                    base: expression,
                                    index: *idx as u32,
                                })
                        })
                        .collect();
                    if components.len() == 1 {
                        // only single element swizzle, like pos.y, just return that component
                        Ok(components[0])
                    } else {
                        Ok(context.function.expressions.append(Expression::Compose {
                            ty: self.module.types.fetch_or_append(Type {
                                name: None,
                                inner: TypeInner::Vector {
                                    kind,
                                    width,
                                    size: match components.len() {
                                        2 => VectorSize::Bi,
                                        3 => VectorSize::Tri,
                                        4 => VectorSize::Quad,
                                        _ => {
                                            return Err(ErrorKind::SemanticError(
                                                format!(
                                                    "Bad swizzle size for \"{:?}\": {:?}",
                                                    name, v
                                                )
                                                .into(),
                                            ));
                                        }
                                    },
                                },
                            }),
                            components,
                        }))
                    }
                } else {
                    Err(ErrorKind::SemanticError(
                        format!("Invalid swizzle for vector \"{}\"", name).into(),
                    ))
                }
            }
            _ => Err(ErrorKind::SemanticError(
                format!("Can't lookup field on this type \"{}\"", name).into(),
            )),
        }
    }
}
