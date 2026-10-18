use std::future::Future;
use std::pin::Pin;

use crate::rng::{below, one_to};
use flush_framework::{Command, CommandCtx, CommandResult};

const SIDES_MIN: u64 = 2;
const SIDES_MAX: u64 = 100;

const ANSWERS: &[&str] = &[
    "Com certeza!",
    "É decididamente assim.",
    "Sem dúvida.",
    "Sim, definitivamente.",
    "Pode contar com isso.",
    "Na minha visão, sim.",
    "Muito provável.",
    "As perspectivas são boas.",
    "Sim.",
    "Os sinais apontam que sim.",
    "Resposta nebulosa, tente de novo.",
    "Pergunte novamente mais tarde.",
    "Melhor não te contar agora.",
    "Não dá para prever agora.",
    "Se concentre e pergunte de novo.",
    "Não conte com isso.",
    "Minha resposta é não.",
    "Minhas fontes dizem não.",
    "As perspectivas não são boas.",
    "Muito duvidoso.",
];

pub struct Dado;

impl Command for Dado {
    fn name(&self) -> &str {
        "dado"
    }
    fn description(&self) -> &str {
        "Rola um dado de N lados (padrão: 6)."
    }
    fn usage(&self) -> Option<&str> {
        Some(".dado [lados]")
    }
    fn execute<'a>(
        &'a self,
        ctx: CommandCtx<'a>,
    ) -> Pin<Box<dyn Future<Output = CommandResult> + Send + 'a>> {
        Box::pin(async move {
            let mut sides = 6u64;
            if let Some(raw) = ctx.args.first() {
                match raw.parse::<i64>() {
                    Ok(v) if (SIDES_MIN as i64..=SIDES_MAX as i64).contains(&v) => {
                        sides = v as u64;
                    }
                    _ => {
                        ctx.reply_no_ping(format!(
                            "Use um número de lados entre {SIDES_MIN} e {SIDES_MAX}. Ex.: `.dado 20`."
                        ))
                        .await?;
                        return Ok(());
                    }
                }
            }
            let roll = one_to(sides);
            ctx.reply_text(format!("🎲 Você rolou um d{sides}: **{roll}**!"))
                .await?;
            Ok(())
        })
    }
}

pub struct Moeda;

impl Command for Moeda {
    fn name(&self) -> &str {
        "moeda"
    }
    fn description(&self) -> &str {
        "Joga uma moeda: cara ou coroa."
    }
    fn usage(&self) -> Option<&str> {
        Some(".moeda")
    }
    fn execute<'a>(
        &'a self,
        ctx: CommandCtx<'a>,
    ) -> Pin<Box<dyn Future<Output = CommandResult> + Send + 'a>> {
        Box::pin(async move {
            let cara = below(2) == 0;
            ctx.reply_text(if cara {
                "🪙 **Cara!**"
            } else {
                "🪙 **Coroa!**"
            })
            .await?;
            Ok(())
        })
    }
}

pub struct Ball8;

impl Command for Ball8 {
    fn name(&self) -> &str {
        "8ball"
    }
    fn description(&self) -> &str {
        "Responde sua pergunta como uma bola 8 mágica."
    }
    fn usage(&self) -> Option<&str> {
        Some(".8ball <pergunta>")
    }
    fn execute<'a>(
        &'a self,
        ctx: CommandCtx<'a>,
    ) -> Pin<Box<dyn Future<Output = CommandResult> + Send + 'a>> {
        Box::pin(async move {
            let question = ctx.args.join(" ").trim().to_owned();
            if question.is_empty() {
                ctx.reply_no_ping("Faça uma pergunta. Ex.: `.8ball vou passar na prova?`")
                    .await?;
                return Ok(());
            }
            let answer = ANSWERS[(below(ANSWERS.len() as u64)) as usize];
            ctx.reply_text(format!(
                "🎱 **Pergunta:** {question}\n**Resposta:** {answer}"
            ))
            .await?;
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{ANSWERS, SIDES_MAX, SIDES_MIN};

    #[test]
    fn dice_bounds_match_pinguim() {
        assert_eq!((SIDES_MIN, SIDES_MAX), (2, 100));
    }

    #[test]
    fn ball8_has_twenty_answers() {
        assert_eq!(ANSWERS.len(), 20);
    }
}
