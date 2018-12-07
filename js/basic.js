var app = new PIXI.Application(800, 600, {backgroundColor : 0x1099bb});
document.body.appendChild(app.view);

// create a new Sprite from an image path
var bunny = PIXI.Sprite.fromImage('https://pixijs.io/examples/required/assets/basics/bunny.png')
var carrot = PIXI.Sprite.fromImage('https://vignette.wikia.nocookie.net/herebemonsters/images/d/dd/Carrot-Sprite.png/revision/latest?cb=20140326230404')

// center the sprite's anchor point
bunny.anchor.set(0.5);
carrot.anchor.set(1);

// move the sprite to the center of the screen

carrot.scale.x = 0.2;
carrot.scale.y = 0.2;
carrot.x = app.screen.width / 3;
carrot.y = app.screen.height / 3;
bunny.x = app.screen.width / 2;
bunny.y = app.screen.height / 2;

app.stage.addChild(bunny);
app.stage.addChild(carrot);

// Listen for animate update
app.ticker.add(function(delta) {
    // just for fun, let's rotate mr rabbit a little
    // delta is 1 if running at 100% performance
    // creates frame-independent transformation
    // bunny.rotation += 0.01 * delta;
    bunny.x += 0.1 * delta;
});
